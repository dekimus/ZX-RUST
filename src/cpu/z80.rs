//! Núcleo Z80 completo (documentado + no documentado habitual).
//!
//! El tiempo lo avanza siempre el bus: cada acceso (`fetch`, `mem_*`, `io_*`) y cada
//! ciclo interno (`tick`) se emiten en el orden real del Z80, de modo que la
//! contención futura puede aplicarse en el T-state exacto del acceso.
//!
//! Limitaciones conocidas (documentadas, no ocultas):
//! - Los ciclos internos (`tick`) no se marcan como potencialmente contendidos
//!   (en hardware real algunos lo son, p. ej. con IR en el bus).
//! - Flags de INI/IND/OUTI/OUTD y sus variantes con repetición: siguen "The Undocumented Z80
//!   Documented" (sección 4.3). No se aplican correcciones posteriores en las iteraciones
//!   intermedias de INIR/OTIR (hallazgos recientes); sin referencia verificable no se incluyen.
//! - No hay NMI (el 48K no lo cablea).
//! - Respuesta de IM0/IM2 con vector de bus fijo 0xFF (típico del Spectrum).

use super::flags::*;
use super::opcode::{Fields, Prefix};
use super::registers::Registers;
use crate::machine::bus::Bus;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Index {
    Hl,
    Ix,
    Iy,
}

#[derive(Clone, Debug, Default)]
pub struct Z80 {
    pub regs: Registers,
    pub iff1: bool,
    pub iff2: bool,
    /// Modo de interrupción 0, 1 o 2.
    pub im: u8,
    pub halted: bool,
    /// Registro interno MEMPTR/WZ (afecta a flags X/Y de `BIT n,(HL)`).
    pub memptr: u16,
    /// Tras EI no se acepta interrupción hasta terminar la instrucción siguiente.
    ei_pending: bool,
    /// Q: flags escritos por la instrucción anterior (0 si no modificó F). Usado por SCF/CCF.
    q: u8,
    f_written: bool,
}

fn parity(v: u8) -> bool {
    v.count_ones() % 2 == 0
}

fn sz53(v: u8) -> u8 {
    (v & (S | Y | X)) | if v == 0 { Z } else { 0 }
}

fn sz53p(v: u8) -> u8 {
    sz53(v) | if parity(v) { PV } else { 0 }
}

impl Z80 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset del Z80: PC=0, I=R=0, IFF=0, IM 0. AF y SP quedan en 0xFFFF
    /// (valor habitual tras reset; el resto de registros es indefinido en hardware).
    pub fn reset(&mut self) {
        *self = Self::default();
        self.regs.set_af(0xFFFF);
        self.regs.sp = 0xFFFF;
    }

    // ---------- primitivas ----------

    fn setf(&mut self, v: u8) {
        self.regs.f = v;
        self.f_written = true;
    }

    /// Valor de IR en el bus durante ciclos internos (relevante para la contención si I ∈ 0x40..=0x7F).
    fn ir(&self) -> u16 {
        (self.regs.i as u16) << 8 | self.regs.r as u16
    }

    fn inc_r(&mut self) {
        // R incrementa sus 7 bits bajos en cada M1; el bit 7 se conserva.
        self.regs.r = (self.regs.r & 0x80) | (self.regs.r.wrapping_add(1) & 0x7F);
    }

    fn fetch_opcode<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let v = bus.fetch(self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        self.inc_r();
        v
    }

    fn read_pc<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let v = bus.mem_read(self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        v
    }

    fn read_pc16<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let lo = self.read_pc(bus) as u16;
        let hi = self.read_pc(bus) as u16;
        hi << 8 | lo
    }

    fn read16<B: Bus>(&mut self, bus: &mut B, addr: u16) -> u16 {
        let lo = bus.mem_read(addr) as u16;
        let hi = bus.mem_read(addr.wrapping_add(1)) as u16;
        hi << 8 | lo
    }

    fn write16<B: Bus>(&mut self, bus: &mut B, addr: u16, v: u16) {
        bus.mem_write(addr, v as u8);
        bus.mem_write(addr.wrapping_add(1), (v >> 8) as u8);
    }

    fn push<B: Bus>(&mut self, bus: &mut B, v: u16) {
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        bus.mem_write(self.regs.sp, (v >> 8) as u8);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        bus.mem_write(self.regs.sp, v as u8);
    }

    fn pop<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let lo = bus.mem_read(self.regs.sp) as u16;
        self.regs.sp = self.regs.sp.wrapping_add(1);
        let hi = bus.mem_read(self.regs.sp) as u16;
        self.regs.sp = self.regs.sp.wrapping_add(1);
        hi << 8 | lo
    }

    fn idx(&self, ix: Index) -> u16 {
        match ix {
            Index::Hl => self.regs.hl(),
            Index::Ix => self.regs.ix,
            Index::Iy => self.regs.iy,
        }
    }

    fn set_idx(&mut self, ix: Index, v: u16) {
        match ix {
            Index::Hl => self.regs.set_hl(v),
            Index::Ix => self.regs.ix = v,
            Index::Iy => self.regs.iy = v,
        }
    }

    /// Registro de 8 bits (0..=7 salvo 6). H/L se sustituyen por IXH/IXL/IYH/IYL con prefijo.
    fn reg8(&self, idx: u8, ix: Index) -> u8 {
        match idx {
            0 => self.regs.b,
            1 => self.regs.c,
            2 => self.regs.d,
            3 => self.regs.e,
            4 => (self.idx(ix) >> 8) as u8,
            5 => self.idx(ix) as u8,
            _ => self.regs.a,
        }
    }

    fn set_reg8(&mut self, idx: u8, ix: Index, v: u8) {
        match idx {
            0 => self.regs.b = v,
            1 => self.regs.c = v,
            2 => self.regs.d = v,
            3 => self.regs.e = v,
            4 => {
                let w = self.idx(ix);
                self.set_idx(ix, (w & 0x00FF) | (v as u16) << 8);
            }
            5 => {
                let w = self.idx(ix);
                self.set_idx(ix, (w & 0xFF00) | v as u16);
            }
            _ => self.regs.a = v,
        }
    }

    /// Par de 16 bits: 0 BC, 1 DE, 2 HL/IX/IY, 3 SP.
    fn rr(&self, p: u8, ix: Index) -> u16 {
        match p {
            0 => self.regs.bc(),
            1 => self.regs.de(),
            2 => self.idx(ix),
            _ => self.regs.sp,
        }
    }

    fn set_rr(&mut self, p: u8, ix: Index, v: u16) {
        match p {
            0 => self.regs.set_bc(v),
            1 => self.regs.set_de(v),
            2 => self.set_idx(ix, v),
            _ => self.regs.sp = v,
        }
    }

    /// Dirección del operando (HL) o (IX+d)/(IY+d). Con índice lee `d` y gasta `extra` T.
    fn mem_addr<B: Bus>(&mut self, bus: &mut B, ix: Index, extra: u32) -> u16 {
        if ix == Index::Hl {
            return self.regs.hl();
        }
        let d = self.read_pc(bus) as i8;
        bus.tick_at(self.regs.pc.wrapping_sub(1), extra);
        let a = self.idx(ix).wrapping_add(d as i16 as u16);
        self.memptr = a;
        a
    }

    fn cond(&self, cc: u8) -> bool {
        let f = self.regs.f;
        match cc {
            0 => f & Z == 0,
            1 => f & Z != 0,
            2 => f & C == 0,
            3 => f & C != 0,
            4 => f & PV == 0,
            5 => f & PV != 0,
            6 => f & S == 0,
            _ => f & S != 0,
        }
    }

    // ---------- ALU ----------

    fn add8(&mut self, b: u8, carry: u8) {
        let a = self.regs.a;
        let r = a as u16 + b as u16 + carry as u16;
        let r8 = r as u8;
        let mut f = sz53(r8);
        if (a ^ b ^ r8) & 0x10 != 0 {
            f |= H;
        }
        if (a ^ r8) & (b ^ r8) & 0x80 != 0 {
            f |= PV;
        }
        if r > 0xFF {
            f |= C;
        }
        self.regs.a = r8;
        self.setf(f);
    }

    /// Resta con flags; devuelve el resultado (CP no lo almacena).
    fn sub_flags(&mut self, b: u8, carry: u8, xy_from_operand: bool) -> u8 {
        let a = self.regs.a;
        let r = (a as i16) - (b as i16) - (carry as i16);
        let r8 = r as u8;
        let mut f = N | sz53(r8);
        if xy_from_operand {
            f = (f & !(X | Y)) | (b & (X | Y));
        }
        if (a ^ b ^ r8) & 0x10 != 0 {
            f |= H;
        }
        if (a ^ b) & (a ^ r8) & 0x80 != 0 {
            f |= PV;
        }
        if r < 0 {
            f |= C;
        }
        self.setf(f);
        r8
    }

    fn alu(&mut self, op: u8, v: u8) {
        match op {
            0 => self.add8(v, 0),
            1 => self.add8(v, self.regs.f & C),
            2 => self.regs.a = self.sub_flags(v, 0, false),
            3 => self.regs.a = self.sub_flags(v, self.regs.f & C, false),
            4 => {
                self.regs.a &= v;
                self.setf(sz53p(self.regs.a) | H);
            }
            5 => {
                self.regs.a ^= v;
                self.setf(sz53p(self.regs.a));
            }
            6 => {
                self.regs.a |= v;
                self.setf(sz53p(self.regs.a));
            }
            _ => {
                self.sub_flags(v, 0, true);
            }
        }
    }

    fn inc8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        let mut f = (self.regs.f & C) | sz53(r);
        if r & 0x0F == 0 {
            f |= H;
        }
        if r == 0x80 {
            f |= PV;
        }
        self.setf(f);
        r
    }

    fn dec8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        let mut f = (self.regs.f & C) | N | sz53(r);
        if r & 0x0F == 0x0F {
            f |= H;
        }
        if r == 0x7F {
            f |= PV;
        }
        self.setf(f);
        r
    }

    fn daa(&mut self) {
        let a = self.regs.a;
        let f = self.regs.f;
        let mut adj = 0u8;
        let mut carry = f & C != 0;
        if f & H != 0 || a & 0x0F > 9 {
            adj |= 0x06;
        }
        if carry || a > 0x99 {
            adj |= 0x60;
            carry = true;
        }
        let r = if f & N != 0 {
            a.wrapping_sub(adj)
        } else {
            a.wrapping_add(adj)
        };
        let h = if f & N != 0 {
            f & H != 0 && a & 0x0F < 6
        } else {
            a & 0x0F > 9
        };
        let mut nf = (f & N) | sz53p(r);
        if h {
            nf |= H;
        }
        if carry {
            nf |= C;
        }
        self.regs.a = r;
        self.setf(nf);
    }

    fn add16(&mut self, a: u16, b: u16) -> u16 {
        let r = a as u32 + b as u32;
        let r16 = r as u16;
        let mut f = (self.regs.f & (S | Z | PV)) | ((r16 >> 8) as u8 & (X | Y));
        if (a ^ b ^ r16) & 0x1000 != 0 {
            f |= H;
        }
        if r > 0xFFFF {
            f |= C;
        }
        self.setf(f);
        self.memptr = a.wrapping_add(1);
        r16
    }

    fn adc16(&mut self, b: u16) {
        let a = self.regs.hl();
        let c = (self.regs.f & C) as u32;
        let r = a as u32 + b as u32 + c;
        let r16 = r as u16;
        let mut f = ((r16 >> 8) as u8 & (S | X | Y)) | if r16 == 0 { Z } else { 0 };
        if (a ^ b ^ r16) & 0x1000 != 0 {
            f |= H;
        }
        if (a ^ r16) & (b ^ r16) & 0x8000 != 0 {
            f |= PV;
        }
        if r > 0xFFFF {
            f |= C;
        }
        self.memptr = a.wrapping_add(1);
        self.regs.set_hl(r16);
        self.setf(f);
    }

    fn sbc16(&mut self, b: u16) {
        let a = self.regs.hl();
        let c = (self.regs.f & C) as i32;
        let r = a as i32 - b as i32 - c;
        let r16 = r as u16;
        let mut f = N | ((r16 >> 8) as u8 & (S | X | Y)) | if r16 == 0 { Z } else { 0 };
        if (a ^ b ^ r16) & 0x1000 != 0 {
            f |= H;
        }
        if (a ^ b) & (a ^ r16) & 0x8000 != 0 {
            f |= PV;
        }
        if r < 0 {
            f |= C;
        }
        self.memptr = a.wrapping_add(1);
        self.regs.set_hl(r16);
        self.setf(f);
    }

    /// Rotaciones/desplazamientos de CB (y: 0 RLC,1 RRC,2 RL,3 RR,4 SLA,5 SRA,6 SLL,7 SRL).
    fn rot(&mut self, y: u8, v: u8) -> u8 {
        let c_in = self.regs.f & C;
        let (r, c_out) = match y {
            0 => (v.rotate_left(1), v >> 7),
            1 => (v.rotate_right(1), v & 1),
            2 => ((v << 1) | c_in, v >> 7),
            3 => ((v >> 1) | (c_in << 7), v & 1),
            4 => (v << 1, v >> 7),
            5 => ((v >> 1) | (v & 0x80), v & 1),
            6 => ((v << 1) | 1, v >> 7),
            _ => (v >> 1, v & 1),
        };
        self.setf(sz53p(r) | c_out);
        r
    }

    fn bit(&mut self, n: u8, v: u8, xy_src: u8) {
        let zero = v & (1 << n) == 0;
        let mut f = (self.regs.f & C) | H | (xy_src & (X | Y));
        if zero {
            f |= Z | PV;
        }
        if n == 7 && !zero {
            f |= S;
        }
        self.setf(f);
    }

    // ---------- paso de CPU ----------

    /// Ejecuta una instrucción completa (o un ciclo de HALT, o la aceptación de una interrupción).
    pub fn step<B: Bus>(&mut self, bus: &mut B) {
        let allow_int = !self.ei_pending;
        self.ei_pending = false;
        self.f_written = false;
        if allow_int && self.iff1 && bus.interrupt_line() {
            self.accept_interrupt(bus);
            self.q = 0;
            return;
        }
        if self.halted {
            // HALT ejecuta NOPs: M1 sobre la dirección siguiente sin avanzar PC.
            bus.fetch(self.regs.pc);
            self.inc_r();
            self.q = 0;
            return;
        }
        self.execute(bus);
        self.q = if self.f_written { self.regs.f } else { 0 };
    }

    fn accept_interrupt<B: Bus>(&mut self, bus: &mut B) {
        tracing::trace!(target: "zx48::cpu", tstate = bus.tstate(), pc = format_args!("{:04X}", self.regs.pc), event = "INT_ACCEPT", im = self.im, halted = self.halted);
        self.halted = false;
        self.iff1 = false;
        self.iff2 = false;
        self.inc_r();
        // Ciclo de reconocimiento: 7 T (M1 ampliado con 2 estados de espera).
        bus.tick(7);
        let pc = self.regs.pc;
        self.push(bus, pc);
        match self.im {
            2 => {
                let vector = (self.regs.i as u16) << 8 | 0xFF;
                self.regs.pc = self.read16(bus, vector);
            }
            // IM0 con 0xFF en el bus equivale a RST 38h; IM1 va a 0x0038.
            _ => self.regs.pc = 0x0038,
        }
        self.memptr = self.regs.pc;
    }

    fn execute<B: Bus>(&mut self, bus: &mut B) {
        let mut ix = Index::Hl;
        loop {
            let op = self.fetch_opcode(bus);
            match Prefix::from_byte(op) {
                Some(Prefix::Dd) => ix = Index::Ix,
                Some(Prefix::Fd) => ix = Index::Iy,
                Some(Prefix::Ed) => return self.execute_ed(bus),
                Some(Prefix::Cb) => {
                    if ix == Index::Hl {
                        let op = self.fetch_opcode(bus);
                        return self.execute_cb(bus, op);
                    }
                    return self.execute_indexed_cb(bus, ix);
                }
                _ => return self.execute_main(bus, op, ix),
            }
        }
    }

    fn execute_cb<B: Bus>(&mut self, bus: &mut B, op: u8) {
        let f = Fields::new(op);
        if f.z == 6 {
            let addr = self.regs.hl();
            let v = bus.mem_read(addr);
            bus.tick_at(addr, 1);
            if f.x == 1 {
                self.bit(f.y, v, (self.memptr >> 8) as u8);
                return;
            }
            let r = self.cb_apply(f.x, f.y, v);
            bus.mem_write(addr, r);
        } else {
            let v = self.reg8(f.z, Index::Hl);
            if f.x == 1 {
                self.bit(f.y, v, v);
                return;
            }
            let r = self.cb_apply(f.x, f.y, v);
            self.set_reg8(f.z, Index::Hl, r);
        }
    }

    fn cb_apply(&mut self, x: u8, y: u8, v: u8) -> u8 {
        match x {
            0 => self.rot(y, v),
            2 => v & !(1 << y),
            _ => v | (1 << y),
        }
    }

    fn execute_indexed_cb<B: Bus>(&mut self, bus: &mut B, ix: Index) {
        let d = self.read_pc(bus) as i8;
        // El opcode final se lee como memoria normal (sin M1 ni incremento de R).
        let op = self.read_pc(bus);
        bus.tick_at(self.regs.pc.wrapping_sub(1), 2);
        let addr = self.idx(ix).wrapping_add(d as i16 as u16);
        self.memptr = addr;
        let f = Fields::new(op);
        let v = bus.mem_read(addr);
        bus.tick_at(addr, 1);
        if f.x == 1 {
            self.bit(f.y, v, (addr >> 8) as u8);
            return;
        }
        let r = self.cb_apply(f.x, f.y, v);
        bus.mem_write(addr, r);
        if f.z != 6 {
            self.set_reg8(f.z, Index::Hl, r);
        }
    }

    fn execute_main<B: Bus>(&mut self, bus: &mut B, op: u8, ix: Index) {
        let f = Fields::new(op);
        match f.x {
            0 => self.main_x0(bus, f, ix),
            1 => {
                if op == 0x76 {
                    self.halted = true;
                } else if f.z == 6 {
                    let addr = self.mem_addr(bus, ix, 5);
                    let v = bus.mem_read(addr);
                    self.set_reg8(f.y, Index::Hl, v);
                } else if f.y == 6 {
                    let addr = self.mem_addr(bus, ix, 5);
                    let v = self.reg8(f.z, Index::Hl);
                    bus.mem_write(addr, v);
                } else {
                    let v = self.reg8(f.z, ix);
                    self.set_reg8(f.y, ix, v);
                }
            }
            2 => {
                let v = if f.z == 6 {
                    let addr = self.mem_addr(bus, ix, 5);
                    bus.mem_read(addr)
                } else {
                    self.reg8(f.z, ix)
                };
                self.alu(f.y, v);
            }
            _ => self.main_x3(bus, f, ix),
        }
    }

    fn main_x0<B: Bus>(&mut self, bus: &mut B, f: Fields, ix: Index) {
        match f.z {
            0 => match f.y {
                0 => {}
                1 => {
                    std::mem::swap(&mut self.regs.a, &mut self.regs.a_);
                    std::mem::swap(&mut self.regs.f, &mut self.regs.f_);
                    self.f_written = true;
                }
                2 => {
                    bus.tick_at(self.ir(), 1);
                    let d = self.read_pc(bus) as i8;
                    self.regs.b = self.regs.b.wrapping_sub(1);
                    if self.regs.b != 0 {
                        bus.tick_at(self.regs.pc.wrapping_sub(1), 5);
                        self.regs.pc = self.regs.pc.wrapping_add(d as i16 as u16);
                        self.memptr = self.regs.pc;
                    }
                }
                3 => {
                    let d = self.read_pc(bus) as i8;
                    bus.tick_at(self.regs.pc.wrapping_sub(1), 5);
                    self.regs.pc = self.regs.pc.wrapping_add(d as i16 as u16);
                    self.memptr = self.regs.pc;
                }
                y => {
                    let d = self.read_pc(bus) as i8;
                    if self.cond(y - 4) {
                        bus.tick_at(self.regs.pc.wrapping_sub(1), 5);
                        self.regs.pc = self.regs.pc.wrapping_add(d as i16 as u16);
                        self.memptr = self.regs.pc;
                    }
                }
            },
            1 => {
                if f.q == 0 {
                    let nn = self.read_pc16(bus);
                    self.set_rr(f.p, ix, nn);
                } else {
                    bus.tick_at(self.ir(), 7);
                    let hl = self.idx(ix);
                    let rr = self.rr(f.p, ix);
                    let r = self.add16(hl, rr);
                    self.set_idx(ix, r);
                }
            }
            2 => match (f.q, f.p) {
                (0, 0) => {
                    let a = self.regs.bc();
                    bus.mem_write(a, self.regs.a);
                    self.memptr = (self.regs.a as u16) << 8 | (a.wrapping_add(1) & 0xFF);
                }
                (0, 1) => {
                    let a = self.regs.de();
                    bus.mem_write(a, self.regs.a);
                    self.memptr = (self.regs.a as u16) << 8 | (a.wrapping_add(1) & 0xFF);
                }
                (0, 2) => {
                    let nn = self.read_pc16(bus);
                    let v = self.idx(ix);
                    self.write16(bus, nn, v);
                    self.memptr = nn.wrapping_add(1);
                }
                (0, _) => {
                    let nn = self.read_pc16(bus);
                    bus.mem_write(nn, self.regs.a);
                    self.memptr = (self.regs.a as u16) << 8 | (nn.wrapping_add(1) & 0xFF);
                }
                (_, 0) => {
                    let a = self.regs.bc();
                    self.regs.a = bus.mem_read(a);
                    self.memptr = a.wrapping_add(1);
                }
                (_, 1) => {
                    let a = self.regs.de();
                    self.regs.a = bus.mem_read(a);
                    self.memptr = a.wrapping_add(1);
                }
                (_, 2) => {
                    let nn = self.read_pc16(bus);
                    let v = self.read16(bus, nn);
                    self.set_idx(ix, v);
                    self.memptr = nn.wrapping_add(1);
                }
                _ => {
                    let nn = self.read_pc16(bus);
                    self.regs.a = bus.mem_read(nn);
                    self.memptr = nn.wrapping_add(1);
                }
            },
            3 => {
                bus.tick_at(self.ir(), 2);
                let v = self.rr(f.p, ix);
                let r = if f.q == 0 {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                self.set_rr(f.p, ix, r);
            }
            4 | 5 => {
                let dec = f.z == 5;
                if f.y == 6 {
                    let addr = self.mem_addr(bus, ix, 5);
                    let v = bus.mem_read(addr);
                    bus.tick_at(addr, 1);
                    let r = if dec { self.dec8(v) } else { self.inc8(v) };
                    bus.mem_write(addr, r);
                } else {
                    let v = self.reg8(f.y, ix);
                    let r = if dec { self.dec8(v) } else { self.inc8(v) };
                    self.set_reg8(f.y, ix, r);
                }
            }
            6 => {
                if f.y == 6 {
                    if ix == Index::Hl {
                        let n = self.read_pc(bus);
                        bus.mem_write(self.regs.hl(), n);
                    } else {
                        let d = self.read_pc(bus) as i8;
                        let n = self.read_pc(bus);
                        bus.tick_at(self.regs.pc.wrapping_sub(1), 2);
                        let a = self.idx(ix).wrapping_add(d as i16 as u16);
                        self.memptr = a;
                        bus.mem_write(a, n);
                    }
                } else {
                    let n = self.read_pc(bus);
                    self.set_reg8(f.y, ix, n);
                }
            }
            _ => self.rotate_acc_group(f.y),
        }
    }

    fn rotate_acc_group(&mut self, y: u8) {
        let a = self.regs.a;
        let fl = self.regs.f;
        match y {
            0..=3 => {
                let (r, c) = match y {
                    0 => (a.rotate_left(1), a >> 7),
                    1 => (a.rotate_right(1), a & 1),
                    2 => ((a << 1) | (fl & C), a >> 7),
                    _ => ((a >> 1) | ((fl & C) << 7), a & 1),
                };
                self.regs.a = r;
                self.setf((fl & (S | Z | PV)) | (r & (X | Y)) | c);
            }
            4 => self.daa(),
            5 => {
                let r = !a;
                self.regs.a = r;
                self.setf((fl & (S | Z | PV | C)) | H | N | (r & (X | Y)));
            }
            _ => {
                // SCF (6) / CCF (7). XY = (Q ^ F) | A (variante NMOS).
                let xy = ((self.q ^ fl) | a) & (X | Y);
                let mut nf = (fl & (S | Z | PV)) | xy;
                if y == 6 {
                    nf |= C;
                } else {
                    if fl & C != 0 {
                        nf |= H;
                    } else {
                        nf |= C;
                    }
                }
                self.setf(nf);
            }
        }
    }

    fn main_x3<B: Bus>(&mut self, bus: &mut B, f: Fields, ix: Index) {
        match f.z {
            0 => {
                bus.tick_at(self.ir(), 1);
                if self.cond(f.y) {
                    let a = self.pop(bus);
                    self.regs.pc = a;
                    self.memptr = a;
                }
            }
            1 => {
                if f.q == 0 {
                    let v = self.pop(bus);
                    match f.p {
                        0 => self.regs.set_bc(v),
                        1 => self.regs.set_de(v),
                        2 => self.set_idx(ix, v),
                        _ => {
                            self.regs.set_af(v);
                            self.f_written = true;
                        }
                    }
                } else {
                    match f.p {
                        0 => {
                            let a = self.pop(bus);
                            self.regs.pc = a;
                            self.memptr = a;
                        }
                        1 => {
                            let r = &mut self.regs;
                            std::mem::swap(&mut r.b, &mut r.b_);
                            std::mem::swap(&mut r.c, &mut r.c_);
                            std::mem::swap(&mut r.d, &mut r.d_);
                            std::mem::swap(&mut r.e, &mut r.e_);
                            std::mem::swap(&mut r.h, &mut r.h_);
                            std::mem::swap(&mut r.l, &mut r.l_);
                        }
                        2 => self.regs.pc = self.idx(ix),
                        _ => {
                            bus.tick_at(self.ir(), 2);
                            self.regs.sp = self.idx(ix);
                        }
                    }
                }
            }
            2 => {
                let nn = self.read_pc16(bus);
                self.memptr = nn;
                if self.cond(f.y) {
                    self.regs.pc = nn;
                }
            }
            3 => match f.y {
                0 => {
                    let nn = self.read_pc16(bus);
                    self.memptr = nn;
                    self.regs.pc = nn;
                }
                2 => {
                    let n = self.read_pc(bus);
                    let port = (self.regs.a as u16) << 8 | n as u16;
                    bus.io_write(port, self.regs.a);
                    self.memptr = (self.regs.a as u16) << 8 | (n.wrapping_add(1) as u16);
                }
                3 => {
                    let n = self.read_pc(bus);
                    let port = (self.regs.a as u16) << 8 | n as u16;
                    self.memptr = port.wrapping_add(1);
                    self.regs.a = bus.io_read(port);
                }
                4 => {
                    let sp = self.regs.sp;
                    let v = self.read16(bus, sp);
                    bus.tick_at(sp.wrapping_add(1), 1);
                    let w = self.idx(ix);
                    bus.mem_write(sp.wrapping_add(1), (w >> 8) as u8);
                    bus.mem_write(sp, w as u8);
                    bus.tick_at(sp, 2);
                    self.set_idx(ix, v);
                    self.memptr = v;
                }
                5 => {
                    let de = self.regs.de();
                    let hl = self.regs.hl();
                    self.regs.set_de(hl);
                    self.regs.set_hl(de);
                }
                6 => {
                    self.iff1 = false;
                    self.iff2 = false;
                }
                _ => {
                    self.iff1 = true;
                    self.iff2 = true;
                    self.ei_pending = true;
                }
            },
            4 => {
                let nn = self.read_pc16(bus);
                self.memptr = nn;
                if self.cond(f.y) {
                    bus.tick_at(self.regs.pc.wrapping_sub(1), 1);
                    let pc = self.regs.pc;
                    self.push(bus, pc);
                    self.regs.pc = nn;
                }
            }
            5 => {
                if f.q == 0 {
                    bus.tick_at(self.ir(), 1);
                    let v = match f.p {
                        0 => self.regs.bc(),
                        1 => self.regs.de(),
                        2 => self.idx(ix),
                        _ => self.regs.af(),
                    };
                    self.push(bus, v);
                } else {
                    // CALL nn (los prefijos DD/ED/FD se resuelven antes).
                    let nn = self.read_pc16(bus);
                    self.memptr = nn;
                    bus.tick_at(self.regs.pc.wrapping_sub(1), 1);
                    let pc = self.regs.pc;
                    self.push(bus, pc);
                    self.regs.pc = nn;
                }
            }
            6 => {
                let n = self.read_pc(bus);
                self.alu(f.y, n);
            }
            _ => {
                bus.tick_at(self.ir(), 1);
                let pc = self.regs.pc;
                self.push(bus, pc);
                self.regs.pc = (f.y as u16) * 8;
                self.memptr = self.regs.pc;
            }
        }
    }

    fn execute_ed<B: Bus>(&mut self, bus: &mut B) {
        let op = self.fetch_opcode(bus);
        let f = Fields::new(op);
        match f.x {
            1 => match f.z {
                0 => {
                    let bc = self.regs.bc();
                    let v = bus.io_read(bc);
                    self.memptr = bc.wrapping_add(1);
                    self.setf((self.regs.f & C) | sz53p(v));
                    if f.y != 6 {
                        self.set_reg8(f.y, Index::Hl, v);
                    }
                }
                1 => {
                    let bc = self.regs.bc();
                    let v = if f.y == 6 {
                        0
                    } else {
                        self.reg8(f.y, Index::Hl)
                    };
                    bus.io_write(bc, v);
                    self.memptr = bc.wrapping_add(1);
                }
                2 => {
                    bus.tick_at(self.ir(), 7);
                    let rr = self.rr(f.p, Index::Hl);
                    if f.q == 0 {
                        self.sbc16(rr)
                    } else {
                        self.adc16(rr)
                    }
                }
                3 => {
                    let nn = self.read_pc16(bus);
                    self.memptr = nn.wrapping_add(1);
                    if f.q == 0 {
                        let v = self.rr(f.p, Index::Hl);
                        self.write16(bus, nn, v);
                    } else {
                        let v = self.read16(bus, nn);
                        self.set_rr(f.p, Index::Hl, v);
                    }
                }
                4 => {
                    let a = self.regs.a;
                    self.regs.a = 0;
                    self.sub_flags(a, 0, false);
                    self.regs.a = 0u8.wrapping_sub(a);
                }
                5 => {
                    // RETN / RETI: ambos restauran IFF1 desde IFF2.
                    self.iff1 = self.iff2;
                    let a = self.pop(bus);
                    self.regs.pc = a;
                    self.memptr = a;
                }
                6 => self.im = [0, 0, 1, 2][(f.y & 3) as usize],
                _ => match f.y {
                    0 => {
                        bus.tick_at(self.ir(), 1);
                        self.regs.i = self.regs.a;
                    }
                    1 => {
                        bus.tick_at(self.ir(), 1);
                        self.regs.r = self.regs.a;
                    }
                    2 | 3 => {
                        bus.tick_at(self.ir(), 1);
                        let v = if f.y == 2 { self.regs.i } else { self.regs.r };
                        self.regs.a = v;
                        let mut fl = (self.regs.f & C) | sz53(v);
                        if self.iff2 {
                            fl |= PV;
                        }
                        self.setf(fl);
                    }
                    4 | 5 => self.rrd_rld(bus, f.y == 4),
                    _ => {}
                },
            },
            2 if f.y >= 4 && f.z <= 3 => {
                let inc = f.y & 1 == 0;
                let repeat = f.y >= 6;
                match f.z {
                    0 => self.block_ld(bus, inc, repeat),
                    1 => self.block_cp(bus, inc, repeat),
                    2 => self.block_in(bus, inc, repeat),
                    _ => self.block_out(bus, inc, repeat),
                }
            }
            // Resto de ED: NOP de 8 T.
            _ => {}
        }
    }

    fn rrd_rld<B: Bus>(&mut self, bus: &mut B, rrd: bool) {
        let hl = self.regs.hl();
        let m = bus.mem_read(hl);
        bus.tick_at(hl, 4);
        let a = self.regs.a;
        let (na, nm) = if rrd {
            ((a & 0xF0) | (m & 0x0F), (a << 4) | (m >> 4))
        } else {
            ((a & 0xF0) | (m >> 4), (m << 4) | (a & 0x0F))
        };
        bus.mem_write(hl, nm);
        self.regs.a = na;
        self.memptr = hl.wrapping_add(1);
        self.setf((self.regs.f & C) | sz53p(na));
    }

    /// Repetición de instrucciones de bloque: 5 T contendidos con la dirección indicada.
    fn repeat_jump<B: Bus>(&mut self, bus: &mut B, addr: u16) {
        bus.tick_at(addr, 5);
        self.regs.pc = self.regs.pc.wrapping_sub(2);
        self.memptr = self.regs.pc.wrapping_add(1);
    }

    fn step_hl(&mut self, inc: bool) {
        let hl = self.regs.hl();
        self.regs.set_hl(if inc {
            hl.wrapping_add(1)
        } else {
            hl.wrapping_sub(1)
        });
    }

    fn block_ld<B: Bus>(&mut self, bus: &mut B, inc: bool, repeat: bool) {
        let v = bus.mem_read(self.regs.hl());
        let de0 = self.regs.de();
        bus.mem_write(de0, v);
        bus.tick_at(de0, 2);
        self.step_hl(inc);
        let de = self.regs.de();
        self.regs.set_de(if inc {
            de.wrapping_add(1)
        } else {
            de.wrapping_sub(1)
        });
        let bc = self.regs.bc().wrapping_sub(1);
        self.regs.set_bc(bc);
        let n = v.wrapping_add(self.regs.a);
        let mut fl = (self.regs.f & (S | Z | C)) | (n & X) | ((n & 0x02) << 4);
        if bc != 0 {
            fl |= PV;
        }
        if repeat && bc != 0 {
            self.repeat_jump(bus, de0);
            fl = (fl & !(X | Y)) | ((self.regs.pc >> 8) as u8 & (X | Y));
        }
        self.setf(fl);
    }

    fn block_cp<B: Bus>(&mut self, bus: &mut B, inc: bool, repeat: bool) {
        let hl = self.regs.hl();
        let v = bus.mem_read(hl);
        bus.tick_at(hl, 5);
        let a = self.regs.a;
        let r = a.wrapping_sub(v);
        let h = (a ^ v ^ r) & 0x10 != 0;
        self.step_hl(inc);
        self.memptr = if inc {
            self.memptr.wrapping_add(1)
        } else {
            self.memptr.wrapping_sub(1)
        };
        let bc = self.regs.bc().wrapping_sub(1);
        self.regs.set_bc(bc);
        let n = r.wrapping_sub(h as u8);
        let mut fl = (self.regs.f & C) | N | (r & S) | (n & X) | ((n & 0x02) << 4);
        if r == 0 {
            fl |= Z;
        }
        if h {
            fl |= H;
        }
        if bc != 0 {
            fl |= PV;
        }
        if repeat && bc != 0 && r != 0 {
            self.repeat_jump(bus, hl);
            fl = (fl & !(X | Y)) | ((self.regs.pc >> 8) as u8 & (X | Y));
        }
        self.setf(fl);
    }

    fn block_io_flags(&mut self, v: u8, k: u16) {
        let b = self.regs.b;
        let mut fl = sz53(b);
        if v & 0x80 != 0 {
            fl |= N;
        }
        if k > 0xFF {
            fl |= H | C;
        }
        if parity(((k & 7) as u8) ^ b) {
            fl |= PV;
        }
        self.setf(fl);
    }

    fn block_in<B: Bus>(&mut self, bus: &mut B, inc: bool, repeat: bool) {
        bus.tick_at(self.ir(), 1);
        let bc = self.regs.bc();
        let hl0 = self.regs.hl();
        let v = bus.io_read(bc);
        bus.mem_write(self.regs.hl(), v);
        self.memptr = if inc {
            bc.wrapping_add(1)
        } else {
            bc.wrapping_sub(1)
        };
        self.regs.b = self.regs.b.wrapping_sub(1);
        self.step_hl(inc);
        let c = self.regs.c;
        let c2 = if inc {
            c.wrapping_add(1)
        } else {
            c.wrapping_sub(1)
        };
        self.block_io_flags(v, v as u16 + c2 as u16);
        if repeat && self.regs.b != 0 {
            self.repeat_jump(bus, hl0);
        }
    }

    fn block_out<B: Bus>(&mut self, bus: &mut B, inc: bool, repeat: bool) {
        bus.tick_at(self.ir(), 1);
        let v = bus.mem_read(self.regs.hl());
        self.regs.b = self.regs.b.wrapping_sub(1);
        let bc = self.regs.bc();
        bus.io_write(bc, v);
        self.memptr = if inc {
            bc.wrapping_add(1)
        } else {
            bc.wrapping_sub(1)
        };
        self.step_hl(inc);
        self.block_io_flags(v, v as u16 + self.regs.l as u16);
        if repeat && self.regs.b != 0 {
            self.repeat_jump(bus, self.regs.bc());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bus mínimo de 64 KiB planos, sin contención, para probar el CPU aislado.
    struct FlatBus {
        mem: Vec<u8>,
        t: u64,
        int: bool,
    }

    impl FlatBus {
        fn with(program: &[u8]) -> Self {
            let mut mem = vec![0; 0x10000];
            mem[..program.len()].copy_from_slice(program);
            Self {
                mem,
                t: 0,
                int: false,
            }
        }
    }

    impl Bus for FlatBus {
        fn fetch(&mut self, a: u16) -> u8 {
            self.t += 4;
            self.mem[a as usize]
        }
        fn mem_read(&mut self, a: u16) -> u8 {
            self.t += 3;
            self.mem[a as usize]
        }
        fn mem_write(&mut self, a: u16, v: u8) {
            self.t += 3;
            self.mem[a as usize] = v
        }
        fn io_read(&mut self, _: u16) -> u8 {
            self.t += 4;
            0xFF
        }
        fn io_write(&mut self, _: u16, _: u8) {
            self.t += 4
        }
        fn interrupt_line(&self) -> bool {
            self.int
        }
        fn tick(&mut self, n: u32) {
            self.t += n as u64
        }
        fn tstate(&self) -> u64 {
            self.t
        }
    }

    /// Ejecuta `n` instrucciones y devuelve (cpu, bus).
    fn run(program: &[u8], n: usize) -> (Z80, FlatBus) {
        let mut cpu = Z80::new();
        let mut bus = FlatBus::with(program);
        for _ in 0..n {
            cpu.step(&mut bus);
        }
        (cpu, bus)
    }

    #[test]
    fn reset_state() {
        let mut cpu = Z80::new();
        cpu.regs.pc = 0x1234;
        cpu.iff1 = true;
        cpu.reset();
        assert_eq!(cpu.regs.pc, 0);
        assert!(!cpu.iff1 && !cpu.iff2);
        assert_eq!(cpu.im, 0);
    }

    #[test]
    fn ld_a_n_costs_7() {
        let (cpu, bus) = run(&[0x3E, 0x5A], 1);
        assert_eq!(cpu.regs.a, 0x5A);
        assert_eq!(bus.t, 7);
    }

    #[test]
    fn r_register_keeps_bit7() {
        let mut cpu = Z80::new();
        cpu.regs.r = 0xFF;
        let mut bus = FlatBus::with(&[0x00]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.r, 0x80);
    }

    #[test]
    fn halt_spins_in_place() {
        let (cpu, bus) = run(&[0x76], 3);
        assert!(cpu.halted);
        assert_eq!(cpu.regs.pc, 1);
        assert_eq!(bus.t, 12);
    }

    #[test]
    fn ld_hl_indirect_timing() {
        let (cpu, bus) = run(&[0x21, 0x00, 0x80, 0x36, 0x11, 0x46], 3);
        assert_eq!(cpu.regs.b, 0x11);
        assert_eq!(bus.t, 27);
    }

    #[test]
    fn add_overflow_flags() {
        // LD A,0x7F; ADD A,1
        let (cpu, _) = run(&[0x3E, 0x7F, 0xC6, 0x01], 2);
        assert_eq!(cpu.regs.a, 0x80);
        assert_eq!(cpu.regs.f, S | H | PV);
    }

    #[test]
    fn sub_borrow_flags() {
        // LD A,0; SUB 1
        let (cpu, _) = run(&[0x3E, 0x00, 0xD6, 0x01], 2);
        assert_eq!(cpu.regs.a, 0xFF);
        assert_eq!(cpu.regs.f, S | Y | X | H | N | C);
    }

    #[test]
    fn daa_after_add() {
        // LD A,0x15; ADD A,0x27; DAA -> 0x42
        let (cpu, _) = run(&[0x3E, 0x15, 0xC6, 0x27, 0x27], 3);
        assert_eq!(cpu.regs.a, 0x42);
    }

    #[test]
    fn indexed_load_timing_and_value() {
        // LD IX,0x9000 ; LD (IX+5),0x77 ; LD A,(IX+5)
        let mut cpu = Z80::new();
        let mut bus = FlatBus::with(&[
            0xDD, 0x21, 0x00, 0x90, 0xDD, 0x36, 0x05, 0x77, 0xDD, 0x7E, 0x05,
        ]);
        cpu.step(&mut bus);
        assert_eq!(bus.t, 14);
        cpu.step(&mut bus);
        assert_eq!(bus.t, 14 + 19);
        cpu.step(&mut bus);
        assert_eq!(bus.t, 14 + 19 + 19);
        assert_eq!(cpu.regs.a, 0x77);
    }

    #[test]
    fn ddcb_timing_and_undocumented_copy() {
        // LD IX,0x9000; (0x9001)=0x01; DD CB 01 00 = RLC (IX+1),B
        let mut cpu = Z80::new();
        let mut bus = FlatBus::with(&[0xDD, 0x21, 0x00, 0x90, 0xDD, 0xCB, 0x01, 0x00]);
        bus.mem[0x9001] = 0x81;
        cpu.step(&mut bus);
        let t0 = bus.t;
        cpu.step(&mut bus);
        assert_eq!(bus.t - t0, 23);
        assert_eq!(bus.mem[0x9001], 0x03);
        assert_eq!(cpu.regs.b, 0x03);
        assert_eq!(cpu.regs.f & C, C);
    }

    #[test]
    fn push_pop_call_ret() {
        // LD SP,0x8000; CALL 0x0010; (0x10: RET)
        let mut cpu = Z80::new();
        let mut bus = FlatBus::with(&[0x31, 0x00, 0x80, 0xCD, 0x10, 0x00]);
        bus.mem[0x10] = 0xC9;
        cpu.step(&mut bus);
        let t = bus.t;
        cpu.step(&mut bus);
        assert_eq!(bus.t - t, 17);
        assert_eq!(cpu.regs.pc, 0x10);
        let t = bus.t;
        cpu.step(&mut bus);
        assert_eq!(bus.t - t, 10);
        assert_eq!(cpu.regs.pc, 6);
        assert_eq!(cpu.regs.sp, 0x8000);
    }

    #[test]
    fn ldir_copies_and_times() {
        // LD HL,0x8000; LD DE,0x9000; LD BC,3; LDIR
        let mut cpu = Z80::new();
        let mut bus = FlatBus::with(&[0x21, 0, 0x80, 0x11, 0, 0x90, 0x01, 3, 0, 0xED, 0xB0]);
        bus.mem[0x8000..0x8003].copy_from_slice(&[1, 2, 3]);
        for _ in 0..3 {
            cpu.step(&mut bus);
        }
        let t = bus.t;
        for _ in 0..3 {
            cpu.step(&mut bus);
        }
        assert_eq!(bus.t - t, 21 + 21 + 16);
        assert_eq!(&bus.mem[0x9000..0x9003], &[1, 2, 3]);
        assert_eq!(cpu.regs.bc(), 0);
    }

    #[test]
    fn im1_interrupt_enters_0038_in_13_tstates() {
        // LD SP,0x8000; EI; NOP; NOP
        let mut cpu = Z80::new();
        let mut bus = FlatBus::with(&[0x31, 0x00, 0x80, 0xFB, 0x00, 0x00]);
        cpu.im = 1;
        bus.int = true;
        cpu.step(&mut bus); // LD SP
        assert_eq!(cpu.regs.pc, 3);
        cpu.step(&mut bus); // EI
        // Tras EI no se acepta antes de la siguiente instrucción.
        cpu.step(&mut bus); // NOP ejecutado
        assert_eq!(cpu.regs.pc, 5);
        let t = bus.t;
        cpu.step(&mut bus); // aceptación
        assert_eq!(bus.t - t, 13);
        assert_eq!(cpu.regs.pc, 0x0038);
        assert_eq!(cpu.regs.sp, 0x7FFE);
        assert_eq!(bus.mem[0x7FFE], 5);
        assert!(!cpu.iff1);
    }

    #[test]
    fn im2_interrupt_takes_19_tstates() {
        let mut cpu = Z80::new();
        let mut bus = FlatBus::with(&[]);
        cpu.im = 2;
        cpu.iff1 = true;
        cpu.regs.i = 0x90;
        cpu.regs.sp = 0x8000;
        bus.mem[0x90FF] = 0x34;
        bus.mem[0x9100] = 0x12;
        bus.int = true;
        cpu.step(&mut bus);
        assert_eq!(bus.t, 19);
        assert_eq!(cpu.regs.pc, 0x1234);
    }

    #[test]
    fn halt_released_by_interrupt_returns_after_halt() {
        let mut cpu = Z80::new();
        let mut bus = FlatBus::with(&[0x76]);
        cpu.im = 1;
        cpu.iff1 = true;
        cpu.regs.sp = 0x8000;
        cpu.step(&mut bus);
        assert!(cpu.halted);
        bus.int = true;
        cpu.step(&mut bus);
        assert!(!cpu.halted);
        assert_eq!(bus.mem[0x7FFE], 1);
    }
}
