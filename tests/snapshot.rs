//! Snapshots sobre la ROM real: equivalencia de ejecución tras guardar/cargar.

use std::path::Path;
use zx48::{Spectrum48, SpectrumKey, rom::loader};

fn machine() -> Option<Spectrum48> {
    let path = Path::new("48.rom");
    if !path.exists() {
        return None;
    }
    let mut m = Spectrum48::new(loader::load_rom(path, false).unwrap());
    m.reset();
    m.run_frame();
    Some(m)
}

fn booted() -> Option<Spectrum48> {
    let mut m = machine()?;
    for _ in 0..99 {
        m.run_frame();
    }
    Some(m)
}

fn screen_ram(m: &Spectrum48) -> Vec<u8> {
    m.bus.memory.ram()[..6912].to_vec()
}

#[test]
fn z80_v3_snapshot_resumes_identically() {
    let Some(mut a) = booted() else { return };
    // Parar en medio de un frame, en una frontera de instrucción cualquiera.
    a.run_tstates(12_345);
    let bytes = a.save_z80();
    let mut b = Spectrum48::new(loader::load_rom("48.rom".as_ref(), false).unwrap());
    b.load_z80(&bytes).unwrap();
    assert_eq!(b.cpu.regs, a.cpu.regs);
    assert_eq!(b.tstate() % 69_888, a.tstate() % 69_888);
    assert_eq!(b.bus.memory.ram()[..], a.bus.memory.ram()[..]);
    for _ in 0..3 {
        a.run_frame();
        b.run_frame();
    }
    assert_eq!(b.cpu.regs, a.cpu.regs);
    assert_eq!(b.tstate() % 69_888, a.tstate() % 69_888);
    assert_eq!(b.bus.memory.ram()[..], a.bus.memory.ram()[..]);
    assert_eq!(b.framebuffer(), a.framebuffer());
}

#[test]
fn sna_snapshot_restores_a_working_basic() {
    let Some(a) = booted() else { return };
    let sna = a.save_sna().unwrap();
    assert_eq!(sna.len(), 49179);
    let mut b = Spectrum48::new(loader::load_rom("48.rom".as_ref(), false).unwrap());
    b.load_sna(&sna).unwrap();
    // Escribimos PRINT 7*6 en la máquina restaurada y miramos la pantalla.
    use SpectrumKey::*;
    let tap = |m: &mut Spectrum48, keys: &[SpectrumKey]| {
        keys.iter().for_each(|&k| m.key_down(k));
        (0..6).for_each(|_| m.run_frame());
        keys.iter().for_each(|&k| m.key_up(k));
        (0..6).for_each(|_| m.run_frame());
    };
    tap(&mut b, &[P]);
    tap(&mut b, &[N7]);
    tap(&mut b, &[SymbolShift, B]);
    tap(&mut b, &[N6]);
    tap(&mut b, &[Enter]);
    // El resultado "42" cambia la pantalla respecto a la original.
    assert!(
        screen_ram(&b) != screen_ram(&a),
        "pc={:#06X} iff1={} im={}",
        b.cpu.regs.pc,
        b.cpu.iff1,
        b.cpu.im
    );
    let rom = b.bus.memory.rom();
    // Busca el glifo del '4' y '2' en la última zona impresa: simplemente comprobamos que ambos aparecen.
    let glyph = |c: u8| -> Vec<u8> {
        (0..8)
            .map(|i| rom[0x3D00 + (c as usize - 32) * 8 + i])
            .collect()
    };
    let find = |g: &[u8]| {
        (0..24).any(|row| {
            (0..32).any(|col| {
                (0..8).all(|line| {
                    let y = row * 8 + line;
                    let addr = ((y & 0xC0) << 5) | ((y & 7) << 8) | ((y & 0x38) << 2) | col;
                    b.bus.memory.ram()[addr] == g[line]
                })
            })
        })
    };
    assert!(find(&glyph(b'4')) && find(&glyph(b'2')));
}

#[test]
fn scr_round_trip_and_rendering() {
    let Some(mut a) = booted() else { return };
    let scr = a.save_scr();
    assert_eq!(scr.len(), 6912);
    let mut b = Spectrum48::new(loader::load_rom("48.rom".as_ref(), false).unwrap());
    b.load_scr(&scr).unwrap();
    assert!(screen_ram(&b) == scr);
    assert!(b.load_scr(&scr[..100]).is_err());
    // Un .scr artificial: atributos únicos por celda se ven en el framebuffer del siguiente frame.
    let mut custom = vec![0u8; 6912];
    custom[..6144].fill(0xFF);
    for (i, a) in custom[6144..].iter_mut().enumerate() {
        *a = (i % 7 + 1) as u8; // ink 1..7 sobre paper negro
    }
    a.load_scr(&custom).unwrap();
    a.run_frame();
    a.run_frame();
    let fb = a.framebuffer();
    let px = |x: usize, y: usize| &fb[(y * zx48::ula::FB_WIDTH + x) * 4..][..4];
    assert_eq!(px(48 + 8, 48), &zx48::ula::video::color_rgba(2, false));
    assert_eq!(px(48 + 16, 48), &zx48::ula::video::color_rgba(3, false));
}

#[test]
fn unsupported_files_are_rejected_not_half_loaded() {
    let Some(mut m) = machine() else { return };
    let before = m.save_z80();
    assert!(m.load_z80(&[0u8; 10]).is_err());
    assert!(m.load_sna(&[0u8; 100]).is_err());
    assert!(m.load_file("xyz", &[]).is_err());
    // El estado no cambió tras errores.
    assert_eq!(m.save_z80(), before);
    let mut z = before.clone();
    z[34] = 3; // 128K
    assert!(m.load_z80(&z).is_err());
    assert_eq!(m.save_z80(), before);
}
