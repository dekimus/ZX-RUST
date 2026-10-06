//! Depurador y perfilador sobre `Spectrum48`: breakpoints de PC, watchpoints de memoria,
//! ejecución paso a paso, volcados, desensamblado y un muestreo de perfil por instrucción.
//!
//! Todo es determinista y sin E/S: `Debugger::execute` recibe una línea de comando y devuelve
//! el texto de salida, de modo que la CLI interactiva y los tests comparten el mismo código.

use crate::cpu::disasm::disassemble;
use crate::machine::spectrum48::Spectrum48;
use crate::ula::timing::TSTATES_PER_FRAME;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchKind {
    Read,
    Write,
    Access,
}

/// Rango de direcciones vigilado (ambos extremos incluidos).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watch {
    pub start: u16,
    pub end: u16,
    pub kind: WatchKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchHit {
    pub addr: u16,
    pub value: u8,
    pub write: bool,
    pub tstate: u64,
}

impl Watch {
    pub fn matches(&self, addr: u16, write: bool) -> bool {
        (self.start..=self.end).contains(&addr)
            && match self.kind {
                WatchKind::Access => true,
                WatchKind::Read => !write,
                WatchKind::Write => write,
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Breakpoint(u16),
    Watch(WatchHit),
    /// Presupuesto de T-states agotado.
    TStates,
    /// Presupuesto de instrucciones agotado.
    Instructions,
}

#[derive(Default, Debug)]
pub struct Profiler {
    /// pc -> (veces ejecutada, T-states consumidos)
    by_pc: BTreeMap<u16, (u64, u64)>,
    total_tstates: u64,
    total_instructions: u64,
}

impl Profiler {
    fn record(&mut self, pc: u16, tstates: u64) {
        let e = self.by_pc.entry(pc).or_insert((0, 0));
        e.0 += 1;
        e.1 += tstates;
        self.total_tstates += tstates;
        self.total_instructions += 1;
    }
}

#[derive(Default, Debug)]
pub struct Debugger {
    breakpoints: BTreeSet<u16>,
    profiler: Option<Profiler>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CommandOutput {
    pub text: String,
    pub quit: bool,
}

const HELP: &str = "\
Comandos (números: 0x1234, $1234, 1234h o decimal):
  help                      esta ayuda
  regs | r                  registros, flags, IFF/IM y reloj
  step [n] | s [n]          ejecuta n instrucciones (por defecto 1) mostrándolas
  cont [frames] | c         continúa hasta breakpoint/watch (límite en frames, por defecto 500)
  frame | f                 ejecuta hasta el siguiente límite de frame
  break <dir> | b <dir>     breakpoint de ejecución;  breaks: lista
  delete <dir>|all | d      borra breakpoint(s)
  watch <dir> [len] [r|w|rw] | w   watchpoint de memoria (por defecto escritura); watches: lista
  unwatch [all]             borra watchpoints
  mem <dir> [len] | m       volcado hexadecimal
  dis [dir] [n] | u         desensambla n instrucciones (por defecto desde PC, 10)
  poke <dir> <valor>        escribe en memoria (la ROM ignora escrituras)
  set <reg> <valor>         a,f,b,c,d,e,h,l,af,bc,de,hl,ix,iy,sp,pc,i,r,im,iff
  profile on|off|show|reset perfil de instrucciones (T-states por dirección)
  tape                      estado de la cinta
  quit | q                  salir";

fn parse_num(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(h) = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .or_else(|| s.strip_prefix('$'))
    {
        u32::from_str_radix(h, 16).ok()
    } else if let Some(h) = s.strip_suffix('h').or_else(|| s.strip_suffix('H')) {
        u32::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}

fn addr(s: Option<&&str>) -> Result<u16, String> {
    let s = s.ok_or("falta la dirección")?;
    parse_num(s)
        .filter(|&v| v <= 0xFFFF)
        .map(|v| v as u16)
        .ok_or_else(|| format!("dirección inválida: {s}"))
}

fn flags_string(f: u8) -> String {
    "SZ5H3PNC"
        .chars()
        .enumerate()
        .map(|(i, c)| if f & (0x80 >> i) != 0 { c } else { '-' })
        .collect()
}

impl Debugger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn breakpoints(&self) -> impl Iterator<Item = u16> + '_ {
        self.breakpoints.iter().copied()
    }

    pub fn add_breakpoint(&mut self, pc: u16) {
        self.breakpoints.insert(pc);
    }

    pub fn remove_breakpoint(&mut self, pc: u16) -> bool {
        self.breakpoints.remove(&pc)
    }

    pub fn profiling(&self) -> bool {
        self.profiler.is_some()
    }

    pub fn start_profile(&mut self) {
        self.profiler = Some(Profiler::default());
    }

    pub fn stop_profile(&mut self) -> Option<Profiler> {
        self.profiler.take()
    }

    /// Ejecuta hasta un breakpoint, un watchpoint o agotar el presupuesto. La primera instrucción
    /// siempre se ejecuta (así se puede continuar desde un breakpoint).
    pub fn run(
        &mut self,
        m: &mut Spectrum48,
        max_tstates: u64,
        max_instructions: u64,
    ) -> StopReason {
        let start = m.tstate();
        let mut n = 0u64;
        m.bus.take_watch_hit();
        loop {
            let pc = m.cpu.regs.pc;
            let t0 = m.tstate();
            m.step();
            n += 1;
            if let Some(p) = self.profiler.as_mut() {
                p.record(pc, m.tstate() - t0);
            }
            if let Some(hit) = m.bus.take_watch_hit() {
                return StopReason::Watch(hit);
            }
            if self.breakpoints.contains(&m.cpu.regs.pc) {
                return StopReason::Breakpoint(m.cpu.regs.pc);
            }
            if n >= max_instructions {
                return StopReason::Instructions;
            }
            if m.tstate() - start >= max_tstates {
                return StopReason::TStates;
            }
        }
    }

    fn line_at(m: &Spectrum48, pc: u16) -> (String, u16) {
        let mem = &m.bus.memory;
        let (text, len) = disassemble(|a| mem.read(a), pc);
        let bytes: Vec<String> = (0..len)
            .map(|i| format!("{:02X}", mem.read(pc.wrapping_add(i))))
            .collect();
        (format!("{pc:04X}  {:<11} {text}", bytes.join(" ")), len)
    }

    fn describe(reason: StopReason, m: &Spectrum48) -> String {
        let (line, _) = Self::line_at(m, m.cpu.regs.pc);
        match reason {
            StopReason::Breakpoint(pc) => format!("Breakpoint en {pc:04X}\n{line}"),
            StopReason::Watch(h) => format!(
                "Watchpoint: {} {:04X} = {:02X} (tstate {})\n{line}",
                if h.write { "escritura" } else { "lectura" },
                h.addr,
                h.value,
                h.tstate
            ),
            StopReason::TStates => format!("Límite alcanzado\n{line}"),
            StopReason::Instructions => line,
        }
    }

    /// Ejecuta una línea de comando y devuelve su salida.
    pub fn execute(&mut self, m: &mut Spectrum48, line: &str) -> CommandOutput {
        let words: Vec<&str> = line.split_whitespace().collect();
        let mut quit = false;
        let text = match self.command(m, &words, &mut quit) {
            Ok(t) => t,
            Err(e) => format!("error: {e}"),
        };
        CommandOutput { text, quit }
    }

    fn command(
        &mut self,
        m: &mut Spectrum48,
        w: &[&str],
        quit: &mut bool,
    ) -> Result<String, String> {
        let Some(&cmd) = w.first() else {
            return Ok(String::new());
        };
        let args = &w[1..];
        let mut out = String::new();
        match cmd {
            "help" | "h" | "?" => out.push_str(HELP),
            "quit" | "q" | "exit" => *quit = true,
            "regs" | "r" => {
                let r = &m.cpu.regs;
                let _ = writeln!(
                    out,
                    "AF={:04X} BC={:04X} DE={:04X} HL={:04X}  [{}]",
                    r.af(),
                    r.bc(),
                    r.de(),
                    r.hl(),
                    flags_string(r.f)
                );
                let _ = writeln!(
                    out,
                    "AF'={:04X} BC'={:04X} DE'={:04X} HL'={:04X}",
                    r.af_alt(),
                    r.bc_alt(),
                    r.de_alt(),
                    r.hl_alt()
                );
                let _ = writeln!(
                    out,
                    "IX={:04X} IY={:04X} SP={:04X} PC={:04X} I={:02X} R={:02X}",
                    r.ix, r.iy, r.sp, r.pc, r.i, r.r
                );
                let _ = writeln!(
                    out,
                    "IFF1={} IFF2={} IM={} HALT={}",
                    m.cpu.iff1 as u8, m.cpu.iff2 as u8, m.cpu.im, m.cpu.halted as u8
                );
                let _ = writeln!(
                    out,
                    "tstate={} (frame {} +{}) border={}",
                    m.tstate(),
                    m.tstate() / TSTATES_PER_FRAME as u64,
                    m.tstate() % TSTATES_PER_FRAME as u64,
                    m.bus.ula.border()
                );
                out.push_str(&Self::line_at(m, r.pc).0);
            }
            "step" | "s" => {
                let n = args
                    .first()
                    .map_or(Some(1), |a| parse_num(a))
                    .ok_or("número inválido")?;
                for _ in 0..n {
                    let (line, _) = Self::line_at(m, m.cpu.regs.pc);
                    let t0 = m.tstate();
                    let reason = self.run(m, u64::MAX, 1);
                    let _ = writeln!(out, "{line}   ({}T)", m.tstate() - t0);
                    if let StopReason::Watch(_) | StopReason::Breakpoint(_) = reason {
                        out.push_str(&Self::describe(reason, m));
                        out.push('\n');
                        break;
                    }
                }
                out = out.trim_end().to_string();
            }
            "cont" | "c" => {
                let frames = args
                    .first()
                    .map_or(Some(500), |a| parse_num(a))
                    .ok_or("número inválido")?;
                let reason = self.run(m, frames as u64 * TSTATES_PER_FRAME as u64, u64::MAX);
                out.push_str(&Self::describe(reason, m));
            }
            "frame" | "f" => {
                let frame = TSTATES_PER_FRAME as u64;
                let target = (m.tstate() / frame + 1) * frame;
                let reason = self.run(m, target - m.tstate(), u64::MAX);
                m.sync_video();
                out.push_str(&Self::describe(reason, m));
            }
            "break" | "b" => {
                let a = addr(args.first())?;
                self.breakpoints.insert(a);
                let _ = write!(out, "Breakpoint en {a:04X}");
            }
            "breaks" => {
                let l: Vec<String> = self
                    .breakpoints
                    .iter()
                    .map(|a| format!("{a:04X}"))
                    .collect();
                out = if l.is_empty() {
                    "(sin breakpoints)".into()
                } else {
                    l.join(" ")
                };
            }
            "delete" | "d" => {
                if args.first() == Some(&"all") {
                    self.breakpoints.clear();
                    out.push_str("Breakpoints borrados");
                } else {
                    let a = addr(args.first())?;
                    out = if self.breakpoints.remove(&a) {
                        format!("Breakpoint {a:04X} borrado")
                    } else {
                        format!("No hay breakpoint en {a:04X}")
                    };
                }
            }
            "watch" | "w" => {
                let start = addr(args.first())?;
                let len = args.get(1).and_then(|a| parse_num(a)).unwrap_or(1).max(1);
                let kind = match args
                    .get(2)
                    .copied()
                    .or_else(|| args.get(1).copied().filter(|a| parse_num(a).is_none()))
                {
                    Some("r") => WatchKind::Read,
                    Some("rw") | Some("a") => WatchKind::Access,
                    _ => WatchKind::Write,
                };
                let end = (start as u32 + len - 1).min(0xFFFF) as u16;
                m.bus.watches.push(Watch { start, end, kind });
                let _ = write!(out, "Watchpoint {start:04X}-{end:04X} ({kind:?})");
            }
            "watches" => {
                let l: Vec<String> = m
                    .bus
                    .watches
                    .iter()
                    .map(|w| format!("{:04X}-{:04X} {:?}", w.start, w.end, w.kind))
                    .collect();
                out = if l.is_empty() {
                    "(sin watchpoints)".into()
                } else {
                    l.join("\n")
                };
            }
            "unwatch" => {
                m.bus.watches.clear();
                out.push_str("Watchpoints borrados");
            }
            "mem" | "m" => {
                let a = addr(args.first())?;
                let len = args
                    .get(1)
                    .and_then(|a| parse_num(a))
                    .unwrap_or(64)
                    .clamp(1, 0x10000) as u32;
                let mut off = 0u32;
                while off < len {
                    let row = (len - off).min(16);
                    let base = a.wrapping_add(off as u16);
                    let bytes: Vec<u8> = (0..row)
                        .map(|i| m.bus.memory.read(base.wrapping_add(i as u16)))
                        .collect();
                    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
                    let asc: String = bytes
                        .iter()
                        .map(|&b| {
                            if (32..127).contains(&b) {
                                b as char
                            } else {
                                '.'
                            }
                        })
                        .collect();
                    let _ = writeln!(out, "{base:04X}  {:<47}  |{asc}|", hex.join(" "));
                    off += 16;
                }
                out = out.trim_end().to_string();
            }
            "dis" | "u" => {
                let mut a = if args.is_empty() {
                    m.cpu.regs.pc
                } else {
                    addr(args.first())?
                };
                let n = args.get(1).and_then(|a| parse_num(a)).unwrap_or(10);
                for _ in 0..n {
                    let (line, len) = Self::line_at(m, a);
                    let marker = if a == m.cpu.regs.pc { ">" } else { " " };
                    let _ = writeln!(out, "{marker}{line}");
                    a = a.wrapping_add(len);
                }
                out = out.trim_end().to_string();
            }
            "poke" => {
                let a = addr(args.first())?;
                let v = args
                    .get(1)
                    .and_then(|v| parse_num(v))
                    .filter(|&v| v <= 0xFF)
                    .ok_or("valor inválido (0..255)")?;
                m.bus.memory.write(a, v as u8);
                let _ = write!(out, "{a:04X} <- {v:02X}");
            }
            "set" => {
                let name = args
                    .first()
                    .ok_or("falta el registro")?
                    .to_ascii_lowercase();
                let v = args
                    .get(1)
                    .and_then(|v| parse_num(v))
                    .ok_or("valor inválido")?;
                let r = &mut m.cpu.regs;
                match name.as_str() {
                    "a" => r.a = v as u8,
                    "f" => r.f = v as u8,
                    "b" => r.b = v as u8,
                    "c" => r.c = v as u8,
                    "d" => r.d = v as u8,
                    "e" => r.e = v as u8,
                    "h" => r.h = v as u8,
                    "l" => r.l = v as u8,
                    "i" => r.i = v as u8,
                    "r" => r.r = v as u8,
                    "af" => r.set_af(v as u16),
                    "bc" => r.set_bc(v as u16),
                    "de" => r.set_de(v as u16),
                    "hl" => r.set_hl(v as u16),
                    "ix" => r.ix = v as u16,
                    "iy" => r.iy = v as u16,
                    "sp" => r.sp = v as u16,
                    "pc" => r.pc = v as u16,
                    "im" if v <= 2 => m.cpu.im = v as u8,
                    "iff" => {
                        m.cpu.iff1 = v != 0;
                        m.cpu.iff2 = v != 0;
                    }
                    _ => {
                        return Err(format!(
                            "registro desconocido o valor fuera de rango: {name}"
                        ));
                    }
                }
                let _ = write!(out, "{name} = {v:X}");
            }
            "profile" => match args.first().copied() {
                Some("on") => {
                    self.start_profile();
                    out.push_str("Perfil activado");
                }
                Some("off") => {
                    self.profiler = None;
                    out.push_str("Perfil desactivado");
                }
                Some("reset") => {
                    if self.profiler.is_some() {
                        self.start_profile();
                    }
                    out.push_str("Perfil reiniciado");
                }
                Some("show") | None => match &self.profiler {
                    Some(p) => out = Self::report(m, p, 15),
                    None => out.push_str("El perfil no está activado (profile on)"),
                },
                Some(x) => return Err(format!("opción desconocida: {x}")),
            },
            "tape" => {
                let t = &m.bus.tape;
                let _ = write!(
                    out,
                    "cinta: {} bloque={} reproduciendo={} fin={}",
                    if t.has_tape() { "insertada" } else { "no" },
                    t.block_index(),
                    t.is_playing(),
                    t.at_end()
                );
            }
            other => return Err(format!("comando desconocido: {other} (help)")),
        }
        Ok(out)
    }

    /// Informe de perfil: porcentaje de T-states por dirección y por página de 256 bytes.
    pub fn report(m: &Spectrum48, p: &Profiler, top: usize) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} instrucciones, {} T-states",
            p.total_instructions, p.total_tstates
        );
        let mut v: Vec<_> = p.by_pc.iter().collect();
        v.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(a.0.cmp(b.0)));
        let pct = |t: u64| {
            if p.total_tstates == 0 {
                0.0
            } else {
                t as f64 * 100.0 / p.total_tstates as f64
            }
        };
        let _ = writeln!(out, "Por instrucción:");
        for (pc, (n, t)) in v.iter().take(top) {
            let (line, _) = Self::line_at(m, **pc);
            let _ = writeln!(out, "  {:5.1}%  {:>10}T  {:>9}x  {line}", pct(*t), t, n);
        }
        let mut pages: BTreeMap<u16, u64> = BTreeMap::new();
        for (pc, (_, t)) in &p.by_pc {
            *pages.entry(pc & 0xFF00).or_default() += t;
        }
        let mut pv: Vec<_> = pages.into_iter().collect();
        pv.sort_by(|a, b| b.1.cmp(&a.1));
        let _ = writeln!(out, "Por página de 256 bytes:");
        for (page, t) in pv.iter().take(8) {
            let _ = writeln!(out, "  {:5.1}%  {page:04X}-{:04X}", pct(*t), page | 0xFF);
        }
        out.trim_end().to_string()
    }

    /// Informe del perfil activo (si lo hay).
    pub fn profile_report(&self, m: &Spectrum48, top: usize) -> Option<String> {
        self.profiler.as_ref().map(|p| Self::report(m, p, top))
    }
}
