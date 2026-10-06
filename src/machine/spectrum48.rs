use super::bus::Bus;
use super::memory::Memory48;
use crate::audio::beeper::{AudioEvent, Beeper};
use crate::cpu::Z80;
use crate::debug::{Watch, WatchHit};
use crate::error::EmulatorError;
use crate::input::kempston::{JoyButton, Kempston};
use crate::input::keyboard::{Keyboard, SpectrumKey};
use crate::rom::loader::ROM_SIZE;
use crate::snapshot::{self, Snapshot};
use crate::tape::{Playable, TapePlayer};
use crate::ula::contention::{self, HardwareProfile};
use crate::ula::{Ula, floating_bus, timing};

/// Bus de la máquina 48K: memoria, ULA, teclado, beeper y reloj maestro.
/// Es dueño del tiempo: aplica la contención de la ULA en el T-state exacto de cada acceso.
pub struct MachineBus {
    pub memory: Memory48,
    pub ula: Ula,
    pub keyboard: Keyboard,
    pub kempston: Kempston,
    pub beeper: Beeper,
    pub tape: TapePlayer,
    pub profile: HardwareProfile,
    /// Watchpoints de memoria (vacío en uso normal: coste de una comprobación por acceso).
    pub watches: Vec<Watch>,
    watch_hit: Option<WatchHit>,
    tstate: u64,
}

impl MachineBus {
    fn new(memory: Memory48) -> Self {
        Self {
            memory,
            ula: Ula::new(),
            keyboard: Keyboard::new(),
            kempston: Kempston::new(),
            beeper: Beeper::new(),
            tape: TapePlayer::new(),
            profile: HardwareProfile::default(),
            watches: Vec::new(),
            watch_hit: None,
            tstate: 0,
        }
    }

    fn frame_tstate(&self) -> u32 {
        timing::tstate_in_frame(self.tstate)
    }

    /// Detiene la CPU lo que dicte la ULA para un acceso contendido que empieza ahora.
    fn contend_now(&mut self) {
        self.tstate += contention::delay(self.profile, self.frame_tstate()) as u64;
    }

    fn contend_addr(&mut self, addr: u16) {
        if contention::is_contended_address(addr) {
            self.contend_now();
        }
    }

    /// Temporización de un ciclo de E/S de 4 T (Sinclair Wiki, "Contended I/O"):
    /// alto byte en 0x40..=0x7F y/o A0 = 0 hacen que la ULA contienda en distintos T-states.
    fn io_cycle(&mut self, port: u16) {
        let high = contention::is_contended_high_byte((port >> 8) as u8);
        let ula_port = port & 1 == 0;
        match (high, ula_port) {
            // A0 = 0 (puerto ULA), alto byte no contendido: N:1, C:3.
            (false, true) => {
                self.tstate += 1;
                self.contend_now();
                self.tstate += 3;
            }
            // A0 = 1, alto byte no contendido: N:4.
            (false, false) => self.tstate += 4,
            // A0 = 0, alto byte contendido: C:1, C:3.
            (true, true) => {
                self.contend_now();
                self.tstate += 1;
                self.contend_now();
                self.tstate += 3;
            }
            // A0 = 1, alto byte contendido: C:1 ×4.
            (true, false) => {
                for _ in 0..4 {
                    self.contend_now();
                    self.tstate += 1;
                }
            }
        }
    }

    fn check_watch(&mut self, addr: u16, value: u8, write: bool) {
        if self.watch_hit.is_none() && self.watches.iter().any(|w| w.matches(addr, write)) {
            self.watch_hit = Some(WatchHit {
                addr,
                value,
                write,
                tstate: self.tstate,
            });
        }
    }

    /// Entrega (y limpia) el último watchpoint disparado.
    pub fn take_watch_hit(&mut self) -> Option<WatchHit> {
        self.watch_hit.take()
    }

    /// Solo diagnóstico/tests: fija el reloj maestro (la ULA se pondrá al día en el siguiente evento).
    #[doc(hidden)]
    pub fn set_tstate(&mut self, t: u64) {
        self.tstate = t;
    }
}

impl Bus for MachineBus {
    fn fetch(&mut self, addr: u16) -> u8 {
        self.contend_addr(addr);
        self.tstate += 4;
        self.memory.read(addr)
    }
    fn mem_read(&mut self, addr: u16) -> u8 {
        self.contend_addr(addr);
        self.tstate += 3;
        let v = self.memory.read(addr);
        if !self.watches.is_empty() {
            self.check_watch(addr, v, false);
        }
        v
    }
    fn mem_write(&mut self, addr: u16, value: u8) {
        self.contend_addr(addr);
        self.tstate += 3;
        // La ULA debe haber renderizado con la RAM antigua todo lo previo a este instante.
        if (0x4000..=0x5AFF).contains(&addr) {
            self.ula.catch_up(self.tstate, self.memory.ram());
        }
        if !self.watches.is_empty() {
            self.check_watch(addr, value, true);
        }
        self.memory.write(addr, value);
    }
    fn io_read(&mut self, port: u16) -> u8 {
        self.io_cycle(port);
        let v = self.io_read_value(port);
        tracing::trace!(target: "zx48::io", tstate = self.tstate, event = "IN", port = format_args!("{port:04X}"), value = format_args!("{v:02X}"));
        v
    }
    fn io_write(&mut self, port: u16, value: u8) {
        self.io_cycle(port);
        tracing::trace!(target: "zx48::io", tstate = self.tstate, event = "OUT", port = format_args!("{port:04X}"), value = format_args!("{value:02X}"));
        self.io_write_effect(port, value);
    }
    fn interrupt_line(&self) -> bool {
        timing::int_active(self.tstate)
    }
    fn tick(&mut self, tstates: u32) {
        self.tstate += tstates as u64;
    }
    fn tick_at(&mut self, addr: u16, tstates: u32) {
        for _ in 0..tstates {
            self.contend_addr(addr);
            self.tstate += 1;
        }
    }
    fn tstate(&self) -> u64 {
        self.tstate
    }
}

impl MachineBus {
    fn io_read_value(&mut self, port: u16) -> u8 {
        if port & 1 == 0 {
            // La señal EAR de la cinta se evalúa en el instante de la lectura.
            if self.tape.is_playing() {
                let level = self.tape.ear_level(self.tstate);
                self.ula.set_ear_input(level);
            }
            self.ula.read_fe((port >> 8) as u8, &self.keyboard)
        } else if self.kempston.decodes(port) {
            self.kempston.read()
        } else {
            // Puerto no conectado: la CPU muestrea el bus en el último T-state del ciclo.
            let sample = timing::tstate_in_frame(self.tstate - 1);
            let floating = floating_bus::read(self.profile, sample, self.memory.ram());
            if Kempston::in_range(port) {
                // Rango Kempston sin interfaz: botones a 0, D5..D7 del floating bus.
                Kempston::absent_value(floating)
            } else {
                floating
            }
        }
    }
    fn io_write_effect(&mut self, port: u16, value: u8) {
        // La ULA responde a puertos con A0 = 0 (decodificación parcial).
        if port & 1 == 0 {
            self.ula.write_fe(self.tstate, value, self.memory.ram());
            self.beeper.set_level(self.tstate, value & 0x10 != 0);
        }
    }
}

/// Entrada de la rutina LD-BYTES de la ROM estándar 16/48K.
const ROM_LD_BYTES: u16 = 0x0556;

pub struct Spectrum48 {
    pub cpu: Z80,
    pub bus: MachineBus,
    /// Arranca la cinta cuando la ROM entra en LD-BYTES (equivale a pulsar PLAY a tiempo).
    /// Es solo comodidad: la señal EAR sigue siendo la fiel generada por pulsos.
    tape_autoplay: bool,
}

impl Spectrum48 {
    pub fn new(rom: Box<[u8; ROM_SIZE]>) -> Self {
        Self {
            cpu: Z80::new(),
            bus: MachineBus::new(Memory48::new(rom)),
            tape_autoplay: false,
        }
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.bus.tstate = 0;
        self.bus.ula.reset();
        self.bus.beeper.reset();
        self.bus.keyboard.release_all();
        self.bus.kempston.release_all();
        self.bus.tape.rewind();
    }

    pub fn tstate(&self) -> u64 {
        self.bus.tstate
    }

    pub fn key_down(&mut self, key: SpectrumKey) {
        self.bus.keyboard.key_down(key);
    }

    pub fn key_up(&mut self, key: SpectrumKey) {
        self.bus.keyboard.key_up(key);
    }

    /// Conecta o desconecta la interfaz Kempston (periférico opcional, desconectado por defecto).
    pub fn set_kempston(&mut self, enabled: bool) {
        self.bus.kempston.set_enabled(enabled);
    }

    pub fn joy_down(&mut self, b: JoyButton) {
        self.bus.kempston.press(b);
    }

    pub fn joy_up(&mut self, b: JoyButton) {
        self.bus.kempston.release(b);
    }

    /// Inserta una cinta (`Tape` de TAP, `Tzx` o cualquier `Playable`).
    pub fn insert_tape(&mut self, tape: impl Into<Playable>) {
        self.bus.tape.insert(tape);
    }

    pub fn tape_play(&mut self) {
        self.bus.tape.play(self.bus.tstate);
    }

    pub fn tape_stop(&mut self) {
        self.bus.tape.stop();
    }

    pub fn tape_rewind(&mut self) {
        self.bus.tape.rewind();
    }

    pub fn set_tape_autoplay(&mut self, on: bool) {
        self.tape_autoplay = on;
    }

    pub fn set_ear_input(&mut self, level: bool) {
        self.bus.ula.set_ear_input(level);
    }

    /// Transiciones del beeper acumuladas desde la última llamada.
    pub fn drain_audio_events(&mut self) -> Vec<AudioEvent> {
        self.bus.beeper.drain_events()
    }

    /// Flancos de la señal de cinta (el "sonido de carga"), con su T-state real. Primero procesa
    /// todos los flancos hasta el instante actual, aunque ningún `IN` los haya consultado.
    pub fn drain_tape_audio_events(&mut self) -> Vec<AudioEvent> {
        self.bus.tape.ear_level(self.bus.tstate);
        self.bus.tape.drain_edges()
    }

    /// Emite la traza de la instrucción que está a punto de ejecutarse (solo con trace CPU activo).
    fn trace_instruction(&self) {
        let r = &self.cpu.regs;
        let mem = &self.bus.memory;
        let (text, _) = crate::cpu::disasm::disassemble(|a| mem.read(a), r.pc);
        tracing::trace!(
            target: "zx48::cpu",
            tstate = self.bus.tstate,
            pc = format_args!("{:04X}", r.pc),
            instr = %text,
            af = format_args!("{:04X}", r.af()),
            bc = format_args!("{:04X}", r.bc()),
            de = format_args!("{:04X}", r.de()),
            hl = format_args!("{:04X}", r.hl()),
            sp = format_args!("{:04X}", r.sp),
            ix = format_args!("{:04X}", r.ix),
            iy = format_args!("{:04X}", r.iy),
            iff = self.cpu.iff1,
            halted = self.cpu.halted,
        );
    }

    /// Estado actual como `Snapshot`. Si la CPU está en HALT, PC apunta al propio HALT para que
    /// al restaurar se vuelva a ejecutar (los formatos no guardan el estado "halted").
    pub fn snapshot(&self) -> Snapshot {
        let mut regs = self.cpu.regs;
        if self.cpu.halted {
            regs.pc = regs.pc.wrapping_sub(1);
        }
        let mut ram = Box::new([0u8; super::memory::RAM_SIZE]);
        ram.copy_from_slice(self.bus.memory.ram());
        Snapshot {
            regs,
            iff1: self.cpu.iff1,
            iff2: self.cpu.iff2,
            im: self.cpu.im,
            border: self.bus.ula.border(),
            ram,
            frame_tstate: Some(timing::tstate_in_frame(self.bus.tstate)),
        }
    }

    /// Sustituye el estado de CPU, RAM, border y fase del frame (ULA y beeper se reinician;
    /// teclado, cinta y ROM se conservan). Sin fase guardada, el frame empieza en 0.
    pub fn load_snapshot(&mut self, s: &Snapshot) {
        tracing::debug!(target: "zx48::snapshot", event = "SNAPSHOT_APPLY", pc = format_args!("{:04X}", s.regs.pc), frame_tstate = ?s.frame_tstate);
        self.cpu = Z80::new();
        self.cpu.regs = s.regs;
        self.cpu.iff1 = s.iff1;
        self.cpu.iff2 = s.iff2;
        self.cpu.im = s.im;
        self.bus.memory.load_ram(&s.ram);
        self.bus.ula.reset();
        self.bus.ula.set_border_immediate(s.border);
        self.bus.beeper.reset();
        self.bus.tstate = s.frame_tstate.unwrap_or(0) as u64;
    }

    /// `.sna` 48K. Falla si la pila no está en RAM.
    pub fn save_sna(&self) -> Result<Vec<u8>, EmulatorError> {
        snapshot::sna::build(&self.snapshot())
    }

    pub fn load_sna(&mut self, data: &[u8]) -> Result<(), EmulatorError> {
        tracing::debug!(target: "zx48::snapshot", event = "LOAD_SNA", bytes = data.len());
        let s = snapshot::sna::parse(data)?;
        self.load_snapshot(&s);
        Ok(())
    }

    pub fn save_z80(&self) -> Vec<u8> {
        snapshot::z80::build(&self.snapshot())
    }

    pub fn load_z80(&mut self, data: &[u8]) -> Result<(), EmulatorError> {
        tracing::debug!(target: "zx48::snapshot", event = "LOAD_Z80", bytes = data.len());
        let s = snapshot::z80::parse(data, Some(&self.bus.memory.rom()[..]))?;
        self.load_snapshot(&s);
        Ok(())
    }

    /// Volcado de pantalla (bitmap + atributos) tal como está en RAM.
    pub fn save_scr(&self) -> Vec<u8> {
        self.bus.memory.ram()[..snapshot::scr::SCR_SIZE].to_vec()
    }

    pub fn load_scr(&mut self, data: &[u8]) -> Result<(), EmulatorError> {
        let scr = snapshot::scr::parse(data)?;
        for (i, b) in scr.iter().enumerate() {
            self.bus.memory.write(0x4000 + i as u16, *b);
        }
        Ok(())
    }

    /// Carga según la extensión del fichero (`sna`, `z80`, `scr`).
    pub fn load_file(&mut self, extension: &str, data: &[u8]) -> Result<(), EmulatorError> {
        match extension.to_ascii_lowercase().as_str() {
            "sna" => self.load_sna(data),
            "z80" => self.load_z80(data),
            "scr" => self.load_scr(data),
            e => Err(EmulatorError::UnsupportedSnapshot(format!(
                "extensión desconocida: .{e}"
            ))),
        }
    }

    /// Hash determinista del estado completo (CPU, reloj, border y RAM) para regresión en CI.
    pub fn state_hash(&self) -> u64 {
        let r = &self.cpu.regs;
        let mut buf = Vec::with_capacity(self.bus.memory.ram().len() + 64);
        for w in [
            r.af(),
            r.bc(),
            r.de(),
            r.hl(),
            r.af_alt(),
            r.bc_alt(),
            r.de_alt(),
            r.hl_alt(),
            r.ix,
            r.iy,
            r.sp,
            r.pc,
        ] {
            buf.extend_from_slice(&w.to_le_bytes());
        }
        buf.extend_from_slice(&[
            r.i,
            r.r,
            self.cpu.iff1 as u8,
            self.cpu.iff2 as u8,
            self.cpu.im,
            self.cpu.halted as u8,
            self.bus.ula.border(),
        ]);
        buf.extend_from_slice(&self.bus.tstate.to_le_bytes());
        buf.extend_from_slice(&self.bus.memory.ram()[..]);
        crate::ula::image::fnv1a(&buf)
    }

    /// Hash del último frame completo (framebuffer RGBA).
    pub fn framebuffer_hash(&self) -> u64 {
        crate::ula::image::fnv1a(self.framebuffer())
    }

    /// Renderiza lo pendiente hasta el T-state actual.
    pub fn sync_video(&mut self) {
        self.bus
            .ula
            .catch_up(self.bus.tstate, self.bus.memory.ram());
    }

    /// Último frame completo (RGBA, `FB_WIDTH`×`FB_HEIGHT` con border).
    pub fn framebuffer(&self) -> &[u8] {
        self.bus.ula.framebuffer()
    }

    pub fn frame_count(&self) -> u64 {
        self.bus.ula.frame_count()
    }

    /// Ejecuta hasta el siguiente límite de frame (múltiplo de 69888 T) y deja el frame completo listo.
    pub fn run_frame(&mut self) {
        let frame = timing::TSTATES_PER_FRAME as u64;
        let target = (self.bus.tstate / frame + 1) * frame;
        while self.bus.tstate < target {
            self.step();
        }
        self.sync_video();
    }

    /// Ejecuta una instrucción; devuelve los T-states consumidos.
    pub fn step(&mut self) -> u32 {
        let start = self.bus.tstate;
        if self.tape_autoplay
            && self.cpu.regs.pc == ROM_LD_BYTES
            && self.bus.tape.has_tape()
            && !self.bus.tape.is_playing()
        {
            self.bus.tape.play(start);
        }
        if tracing::enabled!(target: "zx48::cpu", tracing::Level::TRACE) {
            self.trace_instruction();
        }
        self.cpu.step(&mut self.bus);
        (self.bus.tstate - start) as u32
    }

    /// Ejecuta hasta alcanzar al menos `count` T-states más.
    pub fn run_tstates(&mut self, count: u64) {
        let target = self.bus.tstate + count;
        while self.bus.tstate < target {
            self.step();
        }
        self.sync_video();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine_with(program: &[u8]) -> Spectrum48 {
        let mut rom = Box::new([0u8; ROM_SIZE]);
        rom[..program.len()].copy_from_slice(program);
        Spectrum48::new(rom)
    }

    #[test]
    fn nop_takes_4_tstates() {
        let mut m = machine_with(&[0x00]);
        assert_eq!(m.step(), 4);
        assert_eq!(m.tstate(), 4);
        assert_eq!(m.cpu.regs.pc, 1);
    }

    #[test]
    fn program_runs_and_counts_time() {
        // LD A,0x42 (7T); LD B,A (4T); JP 0 (10T)
        let mut m = machine_with(&[0x3E, 0x42, 0x47, 0xC3, 0x00, 0x00]);
        m.step();
        m.step();
        m.step();
        assert_eq!(m.cpu.regs.a, 0x42);
        assert_eq!(m.cpu.regs.b, 0x42);
        assert_eq!(m.cpu.regs.pc, 0);
        assert_eq!(m.tstate(), 21);
    }

    #[test]
    fn run_tstates_reaches_target() {
        let mut m = machine_with(&[]);
        m.run_tstates(100);
        assert_eq!(m.tstate(), 100);
    }

    #[test]
    fn out_fe_sets_border_and_shows_in_framebuffer() {
        // LD A,2 ; OUT (0xFE),A ; JR $
        let mut m = machine_with(&[0x3E, 0x02, 0xD3, 0xFE, 0x18, 0xFE]);
        m.run_frame();
        assert_eq!(m.bus.ula.border(), 2);
        assert_eq!(m.frame_count(), 1);
        let red = crate::ula::video::color_rgba(2, false);
        // OUT efectivo en T=11: el resto del frame, incluida la esquina inferior, es rojo.
        let last = (crate::ula::FB_WIDTH * crate::ula::FB_HEIGHT - 1) * 4;
        assert_eq!(&m.framebuffer()[last..last + 4], &red);
    }

    #[test]
    fn in_fe_reads_keyboard_rows() {
        // LD A,0xFD ; IN A,(0xFE)  -> semifila A,S,D,F,G
        let mut m = machine_with(&[0x3E, 0xFD, 0xDB, 0xFE]);
        m.key_down(SpectrumKey::D);
        m.step();
        assert_eq!(m.step(), 11);
        assert_eq!(m.cpu.regs.a, 0xA0 | 0x1F & !0b100);
    }

    #[test]
    fn in_fe_reflects_ear_input_on_bit6() {
        let mut m = machine_with(&[0x3E, 0xFF, 0xDB, 0xFE]);
        m.set_ear_input(true);
        m.step();
        m.step();
        assert_eq!(m.cpu.regs.a, 0xFF);
    }

    #[test]
    fn out_fe_bit4_produces_audio_events_with_tstate() {
        // LD A,0x10 ; OUT (FE),A ; LD A,2 (border only, speaker off) ; OUT (FE),A
        let mut m = machine_with(&[0x3E, 0x10, 0xD3, 0xFE, 0x3E, 0x02, 0xD3, 0xFE]);
        for _ in 0..4 {
            m.step();
        }
        let ev = m.drain_audio_events();
        assert_eq!(ev.len(), 2);
        assert_eq!((ev[0].tstate, ev[0].level), (7 + 11, true));
        assert_eq!((ev[1].tstate, ev[1].level), (7 + 11 + 7 + 11, false));
        assert!(m.drain_audio_events().is_empty());
    }

    /// Máquina con una instrucción en `pc` (RAM) y el reloj en `t`.
    fn at(pc: u16, code: &[u8], t: u64) -> Spectrum48 {
        let mut m = machine_with(&[]);
        for (i, b) in code.iter().enumerate() {
            m.bus.memory.write(pc.wrapping_add(i as u16), *b);
        }
        m.cpu.regs.pc = pc;
        m.bus.set_tstate(t);
        m
    }

    #[test]
    fn wiki_example_1_contended_code_and_contended_store() {
        // PC=25000, HL=26000, LD (HL),A en el T-state 14335 -> siguiente opcode en 14352.
        let mut m = at(25000, &[0x77], 14335);
        m.cpu.regs.set_hl(26000);
        m.step();
        assert_eq!(m.tstate(), 14352);
    }

    #[test]
    fn wiki_example_2_uncontended_code_contended_store() {
        // PC=40000 (sin contención), HL=26000: termina en 14344.
        let mut m = at(40000, &[0x77], 14335);
        m.cpu.regs.set_hl(26000);
        m.step();
        assert_eq!(m.tstate(), 14344);
    }

    #[test]
    fn no_contention_outside_display_or_in_upper_ram() {
        let mut m = at(0x6000, &[0x00], 100); // frame temprano
        m.step();
        assert_eq!(m.tstate(), 104);
        let mut m = at(0x9000, &[0x00], 14335);
        m.step();
        assert_eq!(m.tstate(), 14339);
    }

    #[test]
    fn contended_internal_cycles_use_the_right_address() {
        // ADD HL,BC son 7 T internos con IR en el bus. Con I=0x40 (contendido) se retrasan;
        // con I=0x00 no. Código en RAM alta para aislar el efecto.
        let mut m = at(0x9000, &[0x09], 14330);
        m.cpu.regs.i = 0x00;
        m.step();
        assert_eq!(m.tstate(), 14330 + 11);
        let mut m = at(0x9000, &[0x09], 14330);
        m.cpu.regs.i = 0x40;
        m.step();
        // fetch hasta 14334; los 7 ciclos internos se contienden uno a uno:
        // 14334(+0) 14335(+6) 14342(+0) 14343(+6) 14350(+0) 14351(+6) 14358(+0) => 14359.
        assert_eq!(m.tstate(), 14359);
    }

    #[test]
    fn io_contention_patterns() {
        // IN A,(n): fetch 4 + lectura de n 3 = 7 T antes del ciclo de E/S.
        // Alto byte no contendido, puerto ULA (A0=0): N:1, C:3. Ciclo empieza en 14334.
        let mut m = at(0x9000, &[0xDB, 0xFE], 14334 - 7);
        m.cpu.regs.a = 0x00;
        m.step();
        assert_eq!(m.tstate(), 14334 + 1 + 6 + 3);
        // Alto byte no contendido, A0=1: N:4 siempre.
        let mut m = at(0x9000, &[0xDB, 0xFF], 14334 - 7);
        m.cpu.regs.a = 0x00;
        m.step();
        assert_eq!(m.tstate(), 14334 + 4);
        // Alto byte contendido, A0=1: C:1 ×4 desde 14335.
        let mut m = at(0x9000, &[0xDB, 0xFF], 14335 - 7);
        m.cpu.regs.a = 0x40;
        m.step();
        assert_eq!(m.tstate(), 14351);
        // Alto byte contendido, A0=0: C:1, C:3 desde 14335.
        let mut m = at(0x9000, &[0xDB, 0xFE], 14335 - 7);
        m.cpu.regs.a = 0x40;
        m.step();
        assert_eq!(m.tstate(), 14335 + 6 + 1 + 0 + 3);
    }

    #[test]
    fn kempston_port_reads_joystick_only_when_connected() {
        // LD A,0 ; IN A,(0x1F)
        let prog = [0x3E, 0x00, 0xDB, 0x1F];
        // Sin interfaz: floating bus en D5..D7 (0xFF en el border) con los bits de botón a 0,
        // para que el software con el flag de interfaz detectada en RAM no vea botones pulsados.
        let mut m = machine_with(&prog);
        m.step();
        m.step();
        assert_eq!(m.cpu.regs.a, 0xE0);
        // Sin interfaz, leyendo durante el fetch de vídeo: byte de vídeo sin D0..D4.
        let mut m = at(0x9000, &[0xDB, 0x1F], 14335 - 7);
        m.bus.memory.write(0x4000, 0xB0);
        m.cpu.regs.a = 0x00;
        m.step();
        assert_eq!(m.cpu.regs.a, 0xA0);
        // Con interfaz y sin pulsar: 0.
        let mut m = machine_with(&prog);
        m.set_kempston(true);
        m.step();
        m.step();
        assert_eq!(m.cpu.regs.a, 0x00);
        // Con interfaz y disparo + derecha.
        let mut m = machine_with(&prog);
        m.set_kempston(true);
        m.joy_down(JoyButton::Fire);
        m.joy_down(JoyButton::Right);
        m.step();
        m.step();
        assert_eq!(m.cpu.regs.a, 0x11);
        // El teclado (puerto 0xFE) no se ve afectado.
        let mut m = machine_with(&[0x3E, 0xFF, 0xDB, 0xFE]);
        m.set_kempston(true);
        m.step();
        m.step();
        assert_eq!(m.cpu.regs.a, 0xBF);
    }

    #[test]
    fn kempston_absent_never_reports_pressed_buttons() {
        // Regresión: con la interfaz desconectada, IN A,(0x1F) no puede devolver ningún bit
        // de botón (D0..D4) en ningún punto del frame. Un 0xFF completo hacía que un .sna
        // guardado con la interfaz presente (flag "interfaz detectada" ya en RAM) se arrancara
        // solo al cargarlo con el joystick desmarcado en el menú.
        let mut samples = 0u32;
        for s in (0..69_888u64).step_by(131) {
            let mut m = at(0x9000, &[0xDB, 0x1F], s);
            m.cpu.regs.a = 0x00;
            m.step();
            assert_eq!(m.cpu.regs.a & 0x1F, 0, "s={s} -> A={:02X}", m.cpu.regs.a);
            samples += 1;
        }
        assert!(samples > 500);
    }

    #[test]
    fn floating_bus_returns_ula_data_on_unattached_port() {
        // IN A,(0xFF) (alto byte 0 => N:4). El ciclo empieza en s y se muestrea en s+3.
        for (s, expect) in [
            (14335u64, 0xB0u8),
            (14336, 0xA0),
            (14337, 0xB1),
            (14338, 0xA1),
        ] {
            let mut m = at(0x9000, &[0xDB, 0xFF], s - 7);
            m.bus.memory.write(0x4000, 0xB0);
            m.bus.memory.write(0x5800, 0xA0);
            m.bus.memory.write(0x4001, 0xB1);
            m.bus.memory.write(0x5801, 0xA1);
            m.cpu.regs.a = 0x00;
            m.step();
            assert_eq!(m.cpu.regs.a, expect, "s={s}");
        }
        // Zona inactiva (border): 0xFF.
        let mut m = at(0x9000, &[0xDB, 0xFF], 1000);
        m.step();
        assert_eq!(m.cpu.regs.a, 0xFF);
    }

    #[test]
    fn tape_audio_events_are_available_without_any_in_instruction() {
        use crate::tape::{Tape, TapeBlock};
        let mut m = machine_with(&[0x18, 0xFE]); // JR $ : nunca lee el puerto
        m.insert_tape(Tape::from_blocks(vec![TapeBlock::with_checksum(
            0xFF,
            &[1],
        )]));
        m.tape_play();
        m.run_tstates(10_000);
        let ev = m.drain_tape_audio_events();
        assert_eq!(
            ev.iter().map(|e| e.tstate).collect::<Vec<_>>()[..3],
            [0, 2168, 4336]
        );
        assert!(ev.len() >= 5);
    }

    #[test]
    fn interrupt_is_accepted_at_frame_start() {
        // IM 1; LD B,0; DJNZ $ (3328 T, evita la ventana /INT del primer frame); EI; HALT
        let mut m = machine_with(&[0xED, 0x56, 0x06, 0x00, 0x10, 0xFE, 0xFB, 0x76]);
        m.reset();
        while m.cpu.regs.pc != 0x0038 {
            m.step();
            assert!(m.tstate() < 70_000 * 2, "la interrupción no llegó");
        }
        let frame = timing::TSTATES_PER_FRAME as u64;
        // Aceptación en [69888, 69888+32) y 13 T de reconocimiento.
        assert!(
            (frame + 13..frame + 32 + 13).contains(&m.tstate()),
            "tstate={}",
            m.tstate()
        );
    }

    #[test]
    fn execution_is_deterministic() {
        let prog = [0x3E, 0x01, 0x06, 0x02, 0xC3, 0x00, 0x00];
        let mut a = machine_with(&prog);
        let mut b = machine_with(&prog);
        a.run_tstates(5000);
        b.run_tstates(5000);
        assert_eq!(a.tstate(), b.tstate());
        assert_eq!(a.cpu.regs, b.cpu.regs);
    }

    #[test]
    fn ld_to_rom_is_ignored_ld_to_ram_works() {
        // LD HL,0x4000; LD (HL),0x99 ; LD HL,0; LD (HL),0x77
        let mut m = machine_with(&[0x21, 0x00, 0x40, 0x36, 0x99, 0x21, 0x00, 0x00, 0x36, 0x77]);
        for _ in 0..4 {
            m.step();
        }
        assert_eq!(m.bus.memory.read(0x4000), 0x99);
        assert_eq!(m.bus.memory.read(0x0000), 0x21);
    }
}
