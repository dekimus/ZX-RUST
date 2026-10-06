//! Desensamblador Z80 (documentado + no documentado habitual). Sin dependencias de la máquina:
//! solo necesita una función de lectura de memoria.
//!
//! Formato: mnemónicos en mayúsculas, hexadecimal `0x..`, saltos relativos con destino absoluto
//! y desplazamientos indexados con signo: `LD A,(IX+5)`, `JR 0x1000`.
//! Un prefijo DD/FD seguido de otro DD/FD/ED no tiene efecto y se muestra como `NOP*` (1 byte).

const R: [&str; 8] = ["B", "C", "D", "E", "H", "L", "(HL)", "A"];
const RP: [&str; 4] = ["BC", "DE", "HL", "SP"];
const RP2: [&str; 4] = ["BC", "DE", "HL", "AF"];
const CC: [&str; 8] = ["NZ", "Z", "NC", "C", "PO", "PE", "P", "M"];
const ALU: [&str; 8] = [
    "ADD A,", "ADC A,", "SUB ", "SBC A,", "AND ", "XOR ", "OR ", "CP ",
];
const ROT: [&str; 8] = ["RLC", "RRC", "RL", "RR", "SLA", "SRA", "SLL", "SRL"];
const BLOCK: [[&str; 4]; 4] = [
    ["LDI", "CPI", "INI", "OUTI"],
    ["LDD", "CPD", "IND", "OUTD"],
    ["LDIR", "CPIR", "INIR", "OTIR"],
    ["LDDR", "CPDR", "INDR", "OTDR"],
];

struct Cursor<F: FnMut(u16) -> u8> {
    read: F,
    pc: u16,
    start: u16,
}

impl<F: FnMut(u16) -> u8> Cursor<F> {
    fn u8(&mut self) -> u8 {
        let v = (self.read)(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }
    fn u16(&mut self) -> u16 {
        let lo = self.u8() as u16;
        lo | (self.u8() as u16) << 8
    }
    fn len(&self) -> u16 {
        self.pc.wrapping_sub(self.start)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Ix {
    None,
    X,
    Y,
}

impl Ix {
    fn name(self) -> &'static str {
        match self {
            Ix::X => "IX",
            Ix::Y => "IY",
            Ix::None => "HL",
        }
    }
}

fn reg(i: u8, ix: Ix) -> String {
    match (i, ix) {
        (4, Ix::X) => "IXH".into(),
        (5, Ix::X) => "IXL".into(),
        (4, Ix::Y) => "IYH".into(),
        (5, Ix::Y) => "IYL".into(),
        _ => R[i as usize].into(),
    }
}

fn rp(p: u8, ix: Ix) -> &'static str {
    if p == 2 { ix.name() } else { RP[p as usize] }
}

fn disp(d: u8, ix: Ix) -> String {
    let d = d as i8;
    format!(
        "({}{}{})",
        ix.name(),
        if d < 0 { "-" } else { "+" },
        d.unsigned_abs()
    )
}

fn rel(pc_after: u16, d: u8) -> u16 {
    pc_after.wrapping_add(d as i8 as i16 as u16)
}

/// Desensambla la instrucción en `pc`. Devuelve (texto, longitud en bytes).
pub fn disassemble(read: impl FnMut(u16) -> u8, pc: u16) -> (String, u16) {
    let mut c = Cursor {
        read,
        pc,
        start: pc,
    };
    let text = decode(&mut c);
    (text, c.len())
}

fn decode<F: FnMut(u16) -> u8>(c: &mut Cursor<F>) -> String {
    let mut ix = Ix::None;
    let op = c.u8();
    let op = match op {
        0xDD | 0xFD => {
            ix = if op == 0xDD { Ix::X } else { Ix::Y };
            let next = (c.read)(c.pc);
            if matches!(next, 0xDD | 0xFD | 0xED) {
                return "NOP*".into();
            }
            c.u8()
        }
        o => o,
    };
    match op {
        0xED if ix == Ix::None => return decode_ed(c),
        0xCB => return decode_cb(c, ix),
        _ => {}
    }
    let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
    let (p, q) = (y >> 1, y & 1);
    // Operando de memoria (HL) o (IX+d): lee el desplazamiento solo cuando corresponde.
    let mem = |c: &mut Cursor<F>| -> String {
        if ix == Ix::None {
            "(HL)".into()
        } else {
            disp(c.u8(), ix)
        }
    };
    match x {
        0 => match z {
            0 => match y {
                0 => "NOP".into(),
                1 => "EX AF,AF'".into(),
                2 => {
                    let d = c.u8();
                    format!("DJNZ 0x{:04X}", rel(c.pc, d))
                }
                3 => {
                    let d = c.u8();
                    format!("JR 0x{:04X}", rel(c.pc, d))
                }
                _ => {
                    let d = c.u8();
                    format!("JR {},0x{:04X}", CC[(y - 4) as usize], rel(c.pc, d))
                }
            },
            1 => {
                if q == 0 {
                    format!("LD {},0x{:04X}", rp(p, ix), c.u16())
                } else {
                    format!("ADD {},{}", ix.name(), rp(p, ix))
                }
            }
            2 => match (q, p) {
                (0, 0) => "LD (BC),A".into(),
                (0, 1) => "LD (DE),A".into(),
                (0, 2) => format!("LD (0x{:04X}),{}", c.u16(), ix.name()),
                (0, _) => format!("LD (0x{:04X}),A", c.u16()),
                (_, 0) => "LD A,(BC)".into(),
                (_, 1) => "LD A,(DE)".into(),
                (_, 2) => format!("LD {},(0x{:04X})", ix.name(), c.u16()),
                _ => format!("LD A,(0x{:04X})", c.u16()),
            },
            3 => format!("{} {}", if q == 0 { "INC" } else { "DEC" }, rp(p, ix)),
            4 | 5 => {
                let m = if z == 4 { "INC" } else { "DEC" };
                if y == 6 {
                    format!("{m} {}", mem(c))
                } else {
                    format!("{m} {}", reg(y, ix))
                }
            }
            6 => {
                if y == 6 {
                    let m = mem(c);
                    format!("LD {m},0x{:02X}", c.u8())
                } else {
                    format!("LD {},0x{:02X}", reg(y, ix), c.u8())
                }
            }
            _ => ["RLCA", "RRCA", "RLA", "RRA", "DAA", "CPL", "SCF", "CCF"][y as usize].into(),
        },
        1 => {
            if op == 0x76 {
                "HALT".into()
            } else if y == 6 {
                let m = mem(c);
                format!("LD {m},{}", R[z as usize])
            } else if z == 6 {
                let m = mem(c);
                format!("LD {},{m}", R[y as usize])
            } else {
                format!("LD {},{}", reg(y, ix), reg(z, ix))
            }
        }
        2 => {
            let src = if z == 6 { mem(c) } else { reg(z, ix) };
            format!("{}{src}", ALU[y as usize])
        }
        _ => match z {
            0 => format!("RET {}", CC[y as usize]),
            1 => {
                if q == 0 {
                    let n = if p == 2 { ix.name() } else { RP2[p as usize] };
                    format!("POP {n}")
                } else {
                    match p {
                        0 => "RET".into(),
                        1 => "EXX".into(),
                        2 => format!("JP ({})", ix.name()),
                        _ => format!("LD SP,{}", ix.name()),
                    }
                }
            }
            2 => format!("JP {},0x{:04X}", CC[y as usize], c.u16()),
            3 => match y {
                0 => format!("JP 0x{:04X}", c.u16()),
                2 => format!("OUT (0x{:02X}),A", c.u8()),
                3 => format!("IN A,(0x{:02X})", c.u8()),
                4 => format!("EX (SP),{}", ix.name()),
                5 => "EX DE,HL".into(),
                6 => "DI".into(),
                _ => "EI".into(),
            },
            4 => format!("CALL {},0x{:04X}", CC[y as usize], c.u16()),
            5 => {
                if q == 0 {
                    let n = if p == 2 { ix.name() } else { RP2[p as usize] };
                    format!("PUSH {n}")
                } else {
                    format!("CALL 0x{:04X}", c.u16())
                }
            }
            6 => format!("{}0x{:02X}", ALU[y as usize], c.u8()),
            _ => format!("RST 0x{:02X}", y * 8),
        },
    }
}

fn decode_cb<F: FnMut(u16) -> u8>(c: &mut Cursor<F>, ix: Ix) -> String {
    // Con índice: DD CB d op (el desplazamiento va antes del opcode).
    let (operand, op) = if ix == Ix::None {
        (None, c.u8())
    } else {
        let d = c.u8();
        (Some(disp(d, ix)), c.u8())
    };
    let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
    let target = match &operand {
        Some(m) => m.clone(),
        None => R[z as usize].into(),
    };
    let copy = if operand.is_some() && z != 6 && x != 1 {
        format!(",{}", R[z as usize])
    } else {
        String::new()
    };
    match x {
        0 => format!("{} {target}{copy}", ROT[y as usize]),
        1 => format!("BIT {y},{target}"),
        2 => format!("RES {y},{target}{copy}"),
        _ => format!("SET {y},{target}{copy}"),
    }
}

fn decode_ed<F: FnMut(u16) -> u8>(c: &mut Cursor<F>) -> String {
    let op = c.u8();
    let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
    let (p, q) = (y >> 1, y & 1);
    match x {
        1 => match z {
            0 => {
                if y == 6 {
                    "IN (C)".into()
                } else {
                    format!("IN {},(C)", R[y as usize])
                }
            }
            1 => {
                if y == 6 {
                    "OUT (C),0".into()
                } else {
                    format!("OUT (C),{}", R[y as usize])
                }
            }
            2 => format!(
                "{} HL,{}",
                if q == 0 { "SBC" } else { "ADC" },
                RP[p as usize]
            ),
            3 => {
                let nn = c.u16();
                if q == 0 {
                    format!("LD (0x{nn:04X}),{}", RP[p as usize])
                } else {
                    format!("LD {},(0x{nn:04X})", RP[p as usize])
                }
            }
            4 => "NEG".into(),
            5 => {
                if y == 1 {
                    "RETI".into()
                } else {
                    "RETN".into()
                }
            }
            6 => format!("IM {}", [0, 0, 1, 2][(y & 3) as usize]),
            _ => [
                "LD I,A", "LD R,A", "LD A,I", "LD A,R", "RRD", "RLD", "NOP*", "NOP*",
            ][y as usize]
                .into(),
        },
        2 if y >= 4 && z <= 3 => BLOCK[(y - 4) as usize][z as usize].into(),
        _ => "NOP*".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dis(bytes: &[u8], pc: u16) -> (String, u16) {
        disassemble(
            |a| bytes.get(a.wrapping_sub(pc) as usize).copied().unwrap_or(0),
            pc,
        )
    }

    #[test]
    fn known_instructions() {
        let cases: &[(&[u8], &str)] = &[
            (&[0x00], "NOP"),
            (&[0x3E, 0x42], "LD A,0x42"),
            (&[0x21, 0x34, 0x12], "LD HL,0x1234"),
            (&[0x76], "HALT"),
            (&[0xC3, 0x00, 0x80], "JP 0x8000"),
            (&[0x18, 0xFE], "JR 0x1000"),
            (&[0x20, 0x05], "JR NZ,0x1007"),
            (&[0x10, 0xFC], "DJNZ 0x0FFE"),
            (&[0xDD, 0x7E, 0x05], "LD A,(IX+5)"),
            (&[0xFD, 0x7E, 0xFD], "LD A,(IY-3)"),
            (&[0xDD, 0x36, 0x05, 0x07], "LD (IX+5),0x07"),
            (&[0xDD, 0x74, 0x05], "LD (IX+5),H"),
            (&[0xDD, 0x66, 0x05], "LD H,(IX+5)"),
            (&[0xDD, 0x64], "LD IXH,IXH"),
            (&[0xDD, 0xE9], "JP (IX)"),
            (&[0xDD, 0x21, 0x00, 0x90], "LD IX,0x9000"),
            (&[0xDD, 0x34, 0x02], "INC (IX+2)"),
            (&[0xDD, 0x86, 0x02], "ADD A,(IX+2)"),
            (&[0xDD, 0x09], "ADD IX,BC"),
            (&[0xDD, 0xE3], "EX (SP),IX"),
            (&[0xCB, 0x7F], "BIT 7,A"),
            (&[0xCB, 0x06], "RLC (HL)"),
            (&[0xFD, 0xCB, 0x02, 0x06], "RLC (IY+2)"),
            (&[0xDD, 0xCB, 0x05, 0xC0], "SET 0,(IX+5),B"),
            (&[0xDD, 0xCB, 0x05, 0x46], "BIT 0,(IX+5)"),
            (&[0xED, 0xB0], "LDIR"),
            (&[0xED, 0xB3], "OTIR"),
            (&[0xED, 0x78], "IN A,(C)"),
            (&[0xED, 0x70], "IN (C)"),
            (&[0xED, 0x79], "OUT (C),A"),
            (&[0xED, 0x43, 0x00, 0x80], "LD (0x8000),BC"),
            (&[0xED, 0x7B, 0x00, 0x80], "LD SP,(0x8000)"),
            (&[0xED, 0x4A], "ADC HL,BC"),
            (&[0xED, 0x56], "IM 1"),
            (&[0xED, 0x5E], "IM 2"),
            (&[0xED, 0x45], "RETN"),
            (&[0xED, 0x4D], "RETI"),
            (&[0xED, 0x00], "NOP*"),
            (&[0xD3, 0xFE], "OUT (0xFE),A"),
            (&[0xDB, 0xFE], "IN A,(0xFE)"),
            (&[0xE3], "EX (SP),HL"),
            (&[0x32, 0x00, 0x40], "LD (0x4000),A"),
            (&[0x2A, 0x00, 0x40], "LD HL,(0x4000)"),
            (&[0xFF], "RST 0x38"),
            (&[0xC9], "RET"),
            (&[0xC0], "RET NZ"),
            (&[0xF5], "PUSH AF"),
            (&[0xC1], "POP BC"),
            (&[0xCD, 0x2B, 0x60], "CALL 0x602B"),
            (&[0xFE, 0x10], "CP 0x10"),
            (&[0x97], "SUB A"),
            (&[0x08], "EX AF,AF'"),
            (&[0xDD, 0xDD, 0x00], "NOP*"),
            (&[0xDD, 0xED, 0x00], "NOP*"),
        ];
        for (bytes, text) in cases {
            let (t, len) = dis(bytes, 0x1000);
            assert_eq!(&t, text, "{bytes:02X?}");
            if !text.starts_with("NOP*") || bytes[0] == 0xED {
                assert_eq!(len as usize, bytes.len(), "{bytes:02X?}");
            }
        }
    }

    #[test]
    fn relative_jumps_wrap_around_address_space() {
        assert_eq!(dis(&[0x18, 0x7E], 0xFFF0).0, "JR 0x0070");
    }

    /// Contrasta la longitud desensamblada con los bytes realmente consumidos por el CPU
    /// para todos los opcodes (base, CB, ED, DD, FD, DDCB, FDCB).
    #[test]
    fn lengths_match_the_cpu_for_every_opcode() {
        use crate::cpu::Z80;
        use crate::machine::bus::Bus;

        struct Probe {
            mem: Vec<u8>,
            max_code_read: Option<u16>,
        }
        impl Probe {
            fn touch(&mut self, a: u16) -> u8 {
                if (0x1000..0x1010).contains(&a) {
                    self.max_code_read = Some(self.max_code_read.map_or(a, |m| m.max(a)));
                }
                self.mem[a as usize]
            }
        }
        impl Bus for Probe {
            fn fetch(&mut self, a: u16) -> u8 {
                self.touch(a)
            }
            fn mem_read(&mut self, a: u16) -> u8 {
                self.touch(a)
            }
            fn mem_write(&mut self, a: u16, v: u8) {
                self.mem[a as usize] = v
            }
            fn io_read(&mut self, _: u16) -> u8 {
                0xFF
            }
            fn io_write(&mut self, _: u16, _: u8) {}
            fn interrupt_line(&self) -> bool {
                false
            }
            fn tick(&mut self, _: u32) {}
            fn tstate(&self) -> u64 {
                0
            }
        }

        let mut prefixes: Vec<Vec<u8>> = vec![vec![]];
        for p in [0xCBu8, 0xED, 0xDD, 0xFD] {
            prefixes.push(vec![p]);
        }
        prefixes.push(vec![0xDD, 0xCB]);
        prefixes.push(vec![0xFD, 0xCB]);
        let mut checked = 0;
        for prefix in &prefixes {
            for op in 0..=255u8 {
                // DDCB/FDCB: d, op ; resto: op directamente.
                let mut code = prefix.clone();
                if prefix.len() == 2 {
                    code.push(0x05); // desplazamiento
                }
                code.push(op);
                // Prefijos encadenados: tratados como NOP* de 1 byte (el CPU los fusiona).
                if prefix.len() == 1
                    && matches!(prefix[0], 0xDD | 0xFD)
                    && matches!(op, 0xDD | 0xFD | 0xED)
                {
                    continue;
                }
                code.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);
                let mut bus = Probe {
                    mem: vec![0; 0x10000],
                    max_code_read: None,
                };
                bus.mem[0x1000..0x1000 + code.len()].copy_from_slice(&code);
                let mut cpu = Z80::new();
                cpu.regs.pc = 0x1000;
                cpu.regs.set_bc(0x8000);
                cpu.regs.set_de(0x8100);
                cpu.regs.set_hl(0x8200);
                cpu.regs.ix = 0x8300;
                cpu.regs.iy = 0x8400;
                cpu.regs.sp = 0x8800;
                cpu.step(&mut bus);
                let cpu_len = bus.max_code_read.map_or(1, |m| m - 0x1000 + 1);
                let mem = bus.mem.clone();
                let (text, len) = disassemble(|a| mem[a as usize], 0x1000);
                // HALT y similares solo leen el opcode; los saltos pueden no leer más de lo que consumen.
                assert_eq!(len, cpu_len, "{:02X?} -> {text}", &code[..code.len() - 4]);
                checked += 1;
            }
        }
        assert!(checked > 1600);
    }
}
