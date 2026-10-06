//! Z80 instruction exerciser (zexdoc/zexall, GPL, de agn453/ZEXALL) bajo un mini-CP/M.
//! Lento: ejecutar con `cargo test --release --test zex -- --ignored --nocapture`.

use zx48::cpu::Z80;
use zx48::machine::bus::Bus;

struct Flat {
    mem: Vec<u8>,
    t: u64,
}

impl Bus for Flat {
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
        false
    }
    fn tick(&mut self, n: u32) {
        self.t += n as u64
    }
    fn tstate(&self) -> u64 {
        self.t
    }
}

fn run_com(path: &str) -> String {
    let data = std::fs::read(path).expect("fichero .com");
    let mut bus = Flat {
        mem: vec![0; 0x10000],
        t: 0,
    };
    bus.mem[0x100..0x100 + data.len()].copy_from_slice(&data);
    // RET en 0x0005 para el trap BDOS.
    bus.mem[5] = 0xC9;
    let mut cpu = Z80::new();
    cpu.regs.pc = 0x100;
    cpu.regs.sp = 0xF000;
    let mut out = String::new();
    loop {
        match cpu.regs.pc {
            0 => break,
            5 => match cpu.regs.c {
                2 => out.push(cpu.regs.e as char),
                9 => {
                    let mut a = cpu.regs.de() as usize;
                    while bus.mem[a] != b'$' {
                        out.push(bus.mem[a] as char);
                        a += 1;
                    }
                }
                _ => {}
            },
            _ => {}
        }
        cpu.step(&mut bus);
    }
    out
}

fn check(path: &str) {
    let out = run_com(path);
    println!("{out}");
    assert!(!out.contains("ERROR"), "fallos en {path}");
    assert_eq!(out.matches("OK").count(), 67, "faltan pruebas en {path}");
}

#[test]
#[ignore]
fn zexdoc() {
    check("tests/data/zexdoc.com");
}

#[test]
#[ignore]
fn zexall() {
    check("tests/data/zexall.com");
}
