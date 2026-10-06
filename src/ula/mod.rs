//! ULA: reloj de frame, generación de vídeo (incluido el border con precisión de T-state),
//! estado del puerto 0xFE (border/MIC/EAR) y pulso /INT.
//!
//! Modelo de render: la ULA se "pone al día" perezosamente hasta un T-state maestro
//! (`catch_up`). El bus la sincroniza justo antes de cualquier evento que cambie lo que
//! se ve (escritura a display RAM, OUT al puerto ULA, fin de frame), así el resultado es
//! idéntico a renderizar T-state a T-state sin pagar ese coste en cada ciclo de CPU.
//!
//! Geometría (convención de AGENTS.md): el T-state 0 de cada línea es el inicio de los
//! 128 T de zona de imagen; a continuación 24 T de borde derecho, 48 T de retrazo y 24 T
//! de borde izquierdo (que se dibuja a la izquierda de la línea *siguiente*).
//! Cada T-state genera 2 píxeles.
//!
//! Incertidumbres explícitas (sin valor "arbitrario" oculto):
//! - El byte de bitmap/atributo de cada celda de 8 píxeles se captura al inicio de su
//!   ventana de 4 T. En hardware la ULA lo lee unos T antes; el desfase fino se revisará
//!   con la contención/floating bus (Fase 4).
//! - Una escritura de CPU a display RAM se considera efectiva al final del ciclo de 3 T.
//! - El cambio de border de un OUT es efectivo al final del ciclo de E/S de 4 T.

pub mod contention;
pub mod floating_bus;
pub mod image;
pub mod timing;
pub mod video;

use crate::input::keyboard::Keyboard;
use crate::machine::memory::RAM_SIZE;
use timing::{
    DISPLAY_SCANLINES, SCANLINES_BEFORE_DISPLAY, TSTATES_PER_FRAME, TSTATES_PER_SCANLINE,
};

pub const BORDER_LEFT: usize = 48;
pub const BORDER_RIGHT: usize = 48;
pub const BORDER_TOP: usize = 48;
pub const BORDER_BOTTOM: usize = 56;
pub const FB_WIDTH: usize = BORDER_LEFT + 256 + BORDER_RIGHT;
pub const FB_HEIGHT: usize = BORDER_TOP + 192 + BORDER_BOTTOM;
/// Primera scanline del frame que aparece en el framebuffer.
const FIRST_VISIBLE_SCANLINE: u32 = SCANLINES_BEFORE_DISPLAY - BORDER_TOP as u32;
/// T-states de línea con imagen + borde derecho; a partir de aquí hay retrazo.
const HSYNC_START: u32 = 152;
/// T-state de línea en el que empieza el borde izquierdo (de la línea siguiente).
const LEFT_BORDER_START: u32 = 200;
/// Los atributos FLASH conmutan cada 16 frames (periodo 32 frames).
const FLASH_FRAMES: u64 = 16;

pub struct Ula {
    border: u8,
    mic: bool,
    ear_output: bool,
    ear_input: bool,
    /// T-state maestro hasta el que se ha renderizado (exclusivo).
    rendered: u64,
    frame_count: u64,
    back: Vec<u8>,
    front: Vec<u8>,
    latch_bitmap: u8,
    latch_attr: u8,
}

impl Default for Ula {
    fn default() -> Self {
        Self::new()
    }
}

impl Ula {
    pub fn new() -> Self {
        Self {
            border: 7,
            mic: false,
            ear_output: false,
            ear_input: false,
            rendered: 0,
            frame_count: 0,
            back: vec![0; FB_WIDTH * FB_HEIGHT * 4],
            front: vec![0; FB_WIDTH * FB_HEIGHT * 4],
            latch_bitmap: 0,
            latch_attr: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Fija el border sin temporización (carga de snapshots).
    pub fn set_border_immediate(&mut self, color: u8) {
        self.border = color & 7;
    }

    pub fn border(&self) -> u8 {
        self.border
    }

    pub fn mic(&self) -> bool {
        self.mic
    }

    pub fn ear_output(&self) -> bool {
        self.ear_output
    }

    /// Nivel presente en la entrada EAR (lo gobernará el reproductor de cinta).
    pub fn set_ear_input(&mut self, level: bool) {
        self.ear_input = level;
    }

    /// Lectura de un puerto ULA (A0 = 0). `high` = A15..A8 selecciona semifilas.
    /// D0..D4 teclado (activo en bajo), D6 = EAR. La ULA maneja D5 y D7 a 1 en este puerto
    /// (comportamiento observado en el 48K; no proviene del floating bus). Los puertos con
    /// A0 = 1 sí devuelven el floating bus (ver `floating_bus`).
    /// Nota: en algunas revisiones de placa (issue 2/3) EAR in también depende de MIC/EAR out;
    /// el perfil "48K PAL" inicial solo modela la entrada.
    pub fn read_fe(&self, high: u8, keyboard: &Keyboard) -> u8 {
        0xA0 | keyboard.read(high) | ((self.ear_input as u8) << 6)
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    pub fn flash_inverted(&self) -> bool {
        (self.frame_count / FLASH_FRAMES) & 1 == 1
    }

    /// Último frame completo, RGBA, `FB_WIDTH`×`FB_HEIGHT`.
    pub fn framebuffer(&self) -> &[u8] {
        &self.front
    }

    /// Escritura al puerto 0xFE efectiva en el T-state maestro `now`.
    /// Renderiza antes lo anterior con el estado viejo.
    pub fn write_fe(&mut self, now: u64, value: u8, ram: &[u8; RAM_SIZE]) {
        self.catch_up(now, ram);
        if value & 7 != self.border {
            tracing::trace!(target: "zx48::ula", tstate = now, event = "BORDER_CHANGE", value = value & 7);
        }
        self.border = value & 7;
        self.mic = value & 0x08 != 0;
        self.ear_output = value & 0x10 != 0;
    }

    /// Renderiza todos los T-states pendientes hasta `now` (exclusivo).
    pub fn catch_up(&mut self, now: u64, ram: &[u8; RAM_SIZE]) {
        while self.rendered < now {
            let tf = (self.rendered % TSTATES_PER_FRAME as u64) as u32;
            self.render_tstate(tf / TSTATES_PER_SCANLINE, tf % TSTATES_PER_SCANLINE, ram);
            if tf == 0 {
                tracing::trace!(target: "zx48::ula", tstate = self.rendered, event = "INT_START");
            } else if tf == timing::FIRST_DISPLAY_TSTATE {
                tracing::trace!(target: "zx48::video", tstate = self.rendered, event = "FRAME_VISIBLE_START");
            }
            self.rendered += 1;
            if tf == TSTATES_PER_FRAME - 1 {
                tracing::trace!(target: "zx48::video", tstate = self.rendered, event = "FRAME_END", frame = self.frame_count);
                std::mem::swap(&mut self.back, &mut self.front);
                self.frame_count += 1;
            }
        }
    }

    fn put(&mut self, x: usize, row: i64, rgba: [u8; 4]) {
        if row < 0 || row as usize >= FB_HEIGHT || x >= FB_WIDTH {
            return;
        }
        let i = (row as usize * FB_WIDTH + x) * 4;
        self.back[i..i + 4].copy_from_slice(&rgba);
    }

    fn render_tstate(&mut self, scanline: u32, t: u32, ram: &[u8; RAM_SIZE]) {
        if t >= HSYNC_START && t < LEFT_BORDER_START {
            return; // retrazo horizontal: nada visible
        }
        // El borde izquierdo se dibuja al final de la línea anterior a la fila que lo muestra.
        let (row, x0) = if t >= LEFT_BORDER_START {
            (
                scanline as i64 + 1 - FIRST_VISIBLE_SCANLINE as i64,
                ((t - LEFT_BORDER_START) * 2) as usize,
            )
        } else {
            (
                scanline as i64 - FIRST_VISIBLE_SCANLINE as i64,
                BORDER_LEFT + (t * 2) as usize,
            )
        };
        let in_display = (SCANLINES_BEFORE_DISPLAY..SCANLINES_BEFORE_DISPLAY + DISPLAY_SCANLINES)
            .contains(&scanline)
            && t < 128;
        if !in_display {
            let c = video::color_rgba(self.border, false);
            self.put(x0, row, c);
            self.put(x0 + 1, row, c);
            return;
        }
        let y = (scanline - SCANLINES_BEFORE_DISPLAY) as u16;
        let cell = (t / 4) as u16;
        if t % 4 == 0 {
            let b = video::bitmap_address(cell * 8, y) - 0x4000;
            let a = video::attribute_address(cell * 8, y) - 0x4000;
            self.latch_bitmap = ram[b as usize];
            self.latch_attr = ram[a as usize];
        }
        let (ink, paper) = video::attribute_colors(self.latch_attr, self.flash_inverted());
        for p in 0..2u32 {
            let bit = 7 - ((t % 4) * 2 + p);
            let c = if self.latch_bitmap >> bit & 1 != 0 {
                ink
            } else {
                paper
            };
            self.put(x0 + p as usize, row, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: u64 = TSTATES_PER_FRAME as u64;

    fn ram() -> Box<[u8; RAM_SIZE]> {
        Box::new([0; RAM_SIZE])
    }

    fn px(u: &Ula, x: usize, y: usize) -> [u8; 4] {
        let i = (y * FB_WIDTH + x) * 4;
        u.framebuffer()[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn frame_counter_advances_every_69888_tstates() {
        let mut u = Ula::new();
        let r = ram();
        u.catch_up(FRAME - 1, &r);
        assert_eq!(u.frame_count(), 0);
        u.catch_up(FRAME, &r);
        assert_eq!(u.frame_count(), 1);
        u.catch_up(FRAME * 3, &r);
        assert_eq!(u.frame_count(), 3);
    }

    #[test]
    fn uniform_border_fills_everything_but_the_display() {
        let mut u = Ula::new();
        let r = ram();
        u.write_fe(0, 2, &r); // rojo
        u.catch_up(FRAME, &r);
        let red = video::color_rgba(2, false);
        assert_eq!(px(&u, 0, 0), red);
        assert_eq!(px(&u, FB_WIDTH - 1, 0), red);
        assert_eq!(px(&u, 0, FB_HEIGHT - 1), red);
        assert_eq!(px(&u, FB_WIDTH - 1, FB_HEIGHT - 1), red);
        assert_eq!(px(&u, BORDER_LEFT - 1, BORDER_TOP + 10), red);
        assert_eq!(px(&u, BORDER_LEFT + 256, BORDER_TOP + 10), red);
        // La zona de imagen (RAM a cero => papel negro) no es border.
        assert_eq!(px(&u, BORDER_LEFT, BORDER_TOP), video::color_rgba(0, false));
    }

    #[test]
    fn display_pixels_follow_bitmap_and_attributes() {
        let mut u = Ula::new();
        let mut r = ram();
        r[0] = 0b1000_0001; // y=0, celda 0
        r[0x1800] = (1 << 3) | 2; // paper azul, ink rojo
        r[0x100] = 0xFF; // y=1
        u.catch_up(FRAME, &r);
        let (ox, oy) = (BORDER_LEFT, BORDER_TOP);
        assert_eq!(px(&u, ox, oy), video::color_rgba(2, false));
        assert_eq!(px(&u, ox + 1, oy), video::color_rgba(1, false));
        assert_eq!(px(&u, ox + 7, oy), video::color_rgba(2, false));
        assert_eq!(px(&u, ox, oy + 1), video::color_rgba(2, false));
        // celda 1 (attr 0 => ink/paper negros)
        assert_eq!(px(&u, ox + 8, oy), video::color_rgba(0, false));
    }

    #[test]
    fn last_byte_of_screen_lands_bottom_right_of_display() {
        let mut u = Ula::new();
        let mut r = ram();
        r[0x17FF] = 0x01; // (x=248..255, y=191): píxel x=255 encendido
        r[0x1800 + 767] = 0x07; // ink blanco
        u.catch_up(FRAME, &r);
        assert_eq!(
            px(&u, BORDER_LEFT + 255, BORDER_TOP + 191),
            video::color_rgba(7, false)
        );
        assert_eq!(
            px(&u, BORDER_LEFT + 254, BORDER_TOP + 191),
            video::color_rgba(0, false)
        );
    }

    #[test]
    fn border_change_is_effective_at_its_exact_tstate() {
        let mut u = Ula::new();
        let r = ram();
        u.write_fe(0, 1, &r);
        // Scanline 20 (fila 4 del framebuffer), T=50 => píxel x=100 => columna 148.
        let t = 20 * TSTATES_PER_SCANLINE as u64 + 50;
        u.write_fe(t, 2, &r);
        u.catch_up(FRAME, &r);
        assert_eq!(px(&u, 147, 4), video::color_rgba(1, false));
        assert_eq!(px(&u, 148, 4), video::color_rgba(2, false));
        assert_eq!(px(&u, 0, 5), video::color_rgba(2, false));
    }

    #[test]
    fn write_during_frame_only_affects_cells_not_yet_fetched() {
        let mut u = Ula::new();
        let mut r = ram();
        // Línea y=0 empieza en T=14336. Escribimos el bitmap tras la captura de la celda 0 (t=4)
        // pero antes de la de la celda 4 (t=16).
        let now = 14336 + 10;
        u.catch_up(now, &r);
        r[0] = 0xFF; // celda 0, ya capturada: no debe verse
        r[4] = 0xFF; // celda 4, aún no capturada: sí
        r[0x1800] = 7;
        r[0x1804] = 7;
        u.catch_up(FRAME, &r);
        // celda 0 usó bitmap 0 (papel negro) aunque cambió el atributo después de su captura
        assert_eq!(px(&u, BORDER_LEFT, BORDER_TOP), video::color_rgba(0, false));
        assert_eq!(
            px(&u, BORDER_LEFT + 32, BORDER_TOP),
            video::color_rgba(7, false)
        );
    }

    #[test]
    fn flash_inverts_every_16_frames() {
        let mut u = Ula::new();
        let mut r = ram();
        r[0] = 0xFF;
        r[0x1800] = 0x80 | 2; // flash, ink rojo, paper negro
        u.catch_up(FRAME * 2, &r);
        assert!(!u.flash_inverted());
        assert_eq!(px(&u, BORDER_LEFT, BORDER_TOP), video::color_rgba(2, false));
        u.catch_up(FRAME * 17, &r);
        assert!(u.flash_inverted());
        assert_eq!(px(&u, BORDER_LEFT, BORDER_TOP), video::color_rgba(0, false));
        u.catch_up(FRAME * 33, &r);
        assert!(!u.flash_inverted());
    }

    #[test]
    fn read_fe_combines_keyboard_ear_and_fixed_bits() {
        let mut u = Ula::new();
        let mut kb = Keyboard::new();
        assert_eq!(u.read_fe(0xFF, &kb), 0xBF);
        u.set_ear_input(true);
        assert_eq!(u.read_fe(0xFF, &kb), 0xFF);
        kb.key_down(crate::SpectrumKey::A);
        assert_eq!(u.read_fe(0xFD, &kb), 0xFE);
    }

    #[test]
    fn port_fe_latches_mic_and_ear() {
        let mut u = Ula::new();
        let r = ram();
        u.write_fe(0, 0b0001_1101, &r);
        assert_eq!(u.border(), 5);
        assert!(u.mic());
        assert!(u.ear_output());
    }
}
