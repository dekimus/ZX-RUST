//! Floating bus: valor que la ULA coloca en el bus de datos y que la CPU lee de un puerto
//! no conectado (fuente: Sinclair Wiki, "Floating bus").
//!
//! Cada línea de imagen, durante sus 128 T, repite grupos de 8 T-states: bitmap(x), atributo(x),
//! bitmap(x+1), atributo(x+1) y 4 T inactivos (0xFF). Fuera de esa zona (border, retrazo) el
//! bus está inactivo y se lee 0xFF. La tabla de referencia empieza en 14338 (early timing),
//! es decir `contention_start + 3`.

use super::contention::HardwareProfile;
use super::timing::{DISPLAY_SCANLINES, TSTATES_PER_SCANLINE};
use super::video;
use crate::machine::memory::{RAM_SIZE, RAM_START};

/// T-states de la ventana de lectura de una línea.
const ACTIVE_TSTATES: u32 = 128;
/// Desfase entre el primer T contendido y el primer byte colocado en el bus.
const FETCH_OFFSET: u32 = 3;

/// Valor del bus de la ULA en el T-state `frame_tstate` (relativo al inicio de frame).
pub fn read(profile: HardwareProfile, frame_tstate: u32, ram: &[u8; RAM_SIZE]) -> u8 {
    let Some(off) = frame_tstate.checked_sub(profile.contention_start + FETCH_OFFSET) else {
        return 0xFF;
    };
    let line = off / TSTATES_PER_SCANLINE;
    let col = off % TSTATES_PER_SCANLINE;
    if line >= DISPLAY_SCANLINES || col >= ACTIVE_TSTATES {
        return 0xFF;
    }
    let phase = col % 8;
    if phase >= 4 {
        return 0xFF;
    }
    let cell = (col / 8) * 2 + phase / 2;
    let (x, y) = (cell as u16 * 8, line as u16);
    let addr = if phase % 2 == 0 {
        video::bitmap_address(x, y)
    } else {
        video::attribute_address(x, y)
    };
    ram[(addr - RAM_START) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ram() -> Box<[u8; RAM_SIZE]> {
        let mut r = Box::new([0u8; RAM_SIZE]);
        r[0x0000] = 0xB0; // bitmap (0,0)
        r[0x1800] = 0xA0; // atributo celda 0
        r[0x0001] = 0xB1;
        r[0x1801] = 0xA1;
        r[0x0002] = 0xB2;
        r[0x0100] = 0xC0; // bitmap (0, y=1)
        r[0x1802] = 0xA2;
        r
    }

    const P: HardwareProfile = HardwareProfile::EARLY;

    #[test]
    fn first_cycle_matches_reference_table() {
        let r = ram();
        let expected = [
            (14337, 0xFF),
            (14338, 0xB0),
            (14339, 0xA0),
            (14340, 0xB1),
            (14341, 0xA1),
            (14342, 0xFF),
            (14343, 0xFF),
            (14344, 0xFF),
            (14345, 0xFF),
            (14346, 0xB2), // siguiente grupo: celda 2
            (14347, 0xA2),
        ];
        for (t, v) in expected {
            assert_eq!(read(P, t, &r), v, "t={t}");
        }
    }

    #[test]
    fn idle_in_border_and_next_line_uses_next_pixel_row() {
        let r = ram();
        assert_eq!(read(P, 0, &r), 0xFF);
        assert_eq!(read(P, 14338 + 128, &r), 0xFF);
        assert_eq!(read(P, 14338 + 223, &r), 0xFF);
        assert_eq!(read(P, 14338 + 224, &r), 0xC0); // línea 1, bitmap
        assert_eq!(read(P, 14338 + 192 * 224, &r), 0xFF); // tras la última línea
    }

    #[test]
    fn late_profile_shifts_one_tstate() {
        let r = ram();
        assert_eq!(read(HardwareProfile::LATE, 14338, &r), 0xFF);
        assert_eq!(read(HardwareProfile::LATE, 14339, &r), 0xB0);
    }
}
