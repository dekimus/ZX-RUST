//! Smoke test de arranque con la ROM real (se omite si no hay `48.rom`).

use std::path::Path;
use zx48::{Spectrum48, rom::loader};

/// Decodifica la pantalla a texto usando el juego de caracteres de la propia ROM (0x3D00).
fn screen_text(m: &Spectrum48) -> Vec<String> {
    let mem = &m.bus.memory;
    (0..24)
        .map(|row| {
            (0..32)
                .map(|col| {
                    let mut glyph = [0u8; 8];
                    for (line, g) in glyph.iter_mut().enumerate() {
                        let y = row * 8 + line;
                        let addr = 0x4000
                            | ((y & 0xC0) << 5)
                            | ((y & 0x07) << 8)
                            | ((y & 0x38) << 2)
                            | col;
                        *g = mem.read(addr as u16);
                    }
                    (32u8..127)
                        .find(|&c| {
                            let base = 0x3D00 + (c as u16 - 32) * 8;
                            (0..8).all(|i| mem.read(base + i) == glyph[i as usize])
                        })
                        .map_or(' ', |c| c as char)
                })
                .collect()
        })
        .collect()
}

#[test]
fn rom_boots_to_basic_prompt() {
    let path = Path::new("48.rom");
    if !path.exists() {
        eprintln!("48.rom ausente: test omitido");
        return;
    }
    let rom = loader::load_rom(path, false).expect("ROM estándar");
    let mut m = Spectrum48::new(rom);
    m.reset();
    m.run_tstates(69_888 * 200);
    let text = screen_text(&m).join("\n");
    println!("{text}");
    assert!(
        text.contains("1982 Sinclair Research Ltd"),
        "no aparece el copyright:\n{text}"
    );

    // El framebuffer (con border) debe reflejarlo: border blanco y píxeles de texto en negro.
    m.run_frame();
    let fb = m.framebuffer();
    let at = |x: usize, y: usize| &fb[(y * zx48::ula::FB_WIDTH + x) * 4..][..4];
    assert_eq!(at(0, 0), &[0xCD, 0xCD, 0xCD, 0xFF]);
    let ink = (0..zx48::ula::FB_HEIGHT * zx48::ula::FB_WIDTH)
        .filter(|&i| fb[i * 4] == 0)
        .count();
    assert!(ink > 100, "no hay texto en el framebuffer");
}

#[test]
fn boot_is_deterministic() {
    let path = Path::new("48.rom");
    if !path.exists() {
        return;
    }
    let mk = || {
        let mut m = Spectrum48::new(loader::load_rom(path, false).unwrap());
        m.reset();
        m.run_tstates(69_888 * 20);
        m
    };
    let (a, b) = (mk(), mk());
    assert_eq!(a.tstate(), b.tstate());
    assert_eq!(a.cpu.regs, b.cpu.regs);
    assert_eq!(a.bus.memory.ram()[..], b.bus.memory.ram()[..]);
}

fn booted() -> Option<Spectrum48> {
    let path = Path::new("48.rom");
    if !path.exists() {
        return None;
    }
    let mut m = Spectrum48::new(loader::load_rom(path, false).unwrap());
    m.reset();
    m.run_tstates(69_888 * 100);
    Some(m)
}

fn tap_key(m: &mut Spectrum48, keys: &[zx48::SpectrumKey]) {
    for &k in keys {
        m.key_down(k);
    }
    for _ in 0..6 {
        m.run_frame();
    }
    for &k in keys {
        m.key_up(k);
    }
    for _ in 0..6 {
        m.run_frame();
    }
}

#[test]
fn rom_reads_keyboard_and_types_basic_keyword() {
    use zx48::SpectrumKey::*;
    let Some(mut m) = booted() else { return };
    tap_key(&mut m, &[P]); // en modo K la tecla P inserta el token PRINT
    let text = screen_text(&m).join("\n");
    assert!(text.contains("PRINT"), "PRINT no aparece:\n{text}");
}

#[test]
fn basic_print_runs_and_shows_result() {
    use zx48::SpectrumKey::*;
    let Some(mut m) = booted() else { return };
    tap_key(&mut m, &[P]); // PRINT
    tap_key(&mut m, &[N7]);
    tap_key(&mut m, &[SymbolShift, B]); // '*' = SYMBOL SHIFT + B
    tap_key(&mut m, &[N6]);
    tap_key(&mut m, &[Enter]);
    let text = screen_text(&m).join("\n");
    assert!(text.contains("42"), "resultado 42 ausente:\n{text}");
}

/// Teclea `LOAD ""` + Enter (J = LOAD; SYMBOL SHIFT + P = comillas).
fn type_load_quotes(m: &mut Spectrum48) {
    use zx48::SpectrumKey::*;
    tap_key(m, &[J]);
    tap_key(m, &[SymbolShift, P]);
    tap_key(m, &[SymbolShift, P]);
    tap_key(m, &[Enter]);
}

fn basic_program_tape() -> zx48::tape::Tape {
    use zx48::tape::{Tape, TapeBlock, TapeHeaderKind};
    // 10 PRINT 42   (con la forma interna del número: 0x0E + 5 bytes)
    let prog = [
        0x00, 0x0A, 0x0A, 0x00, 0xF5, b'4', b'2', 0x0E, 0x00, 0x00, 0x2A, 0x00, 0x00, 0x0D,
    ];
    Tape::from_blocks(vec![
        TapeBlock::header(
            TapeHeaderKind::Program,
            "demo",
            prog.len() as u16,
            10,
            prog.len() as u16,
        ),
        TapeBlock::with_checksum(0xFF, &prog),
    ])
}

#[test]
fn rom_loads_and_autoruns_basic_program_from_tape_signal() {
    let Some(mut m) = booted() else { return };
    m.insert_tape(basic_program_tape());
    m.set_tape_autoplay(true);
    type_load_quotes(&mut m);
    let mut seen_header = false;
    for _ in 0..1500 {
        m.run_frame();
        let text = screen_text(&m).join("\n");
        seen_header |= text.contains("Program: demo");
        if text.contains("42") {
            assert!(seen_header, "no se vio 'Program: demo':\n{text}");
            assert!(!text.contains("error"), "{text}");
            return;
        }
    }
    panic!("no se cargó el programa:\n{}", screen_text(&m).join("\n"));
}

#[test]
fn corrupt_tape_block_gives_loading_error() {
    let Some(mut m) = booted() else { return };
    let mut tape = basic_program_tape();
    *tape.blocks[1].0.last_mut().unwrap() ^= 0xFF; // checksum de datos erróneo
    m.insert_tape(tape);
    m.set_tape_autoplay(true);
    type_load_quotes(&mut m);
    for _ in 0..1500 {
        m.run_frame();
        let text = screen_text(&m).join("\n");
        if text.contains("R Tape loading error") {
            return;
        }
    }
    panic!(
        "no apareció 'R Tape loading error':\n{}",
        screen_text(&m).join("\n")
    );
}

#[test]
fn demo_tap_file_loads_and_runs() {
    let Some(mut m) = booted() else { return };
    let data = std::fs::read("tests/data/demo.tap").unwrap();
    m.insert_tape(zx48::tape::Tape::from_tap(&data).unwrap());
    m.set_tape_autoplay(true);
    type_load_quotes(&mut m);
    for _ in 0..1500 {
        m.run_frame();
        if screen_text(&m).join("\n").contains("CARGADO DESDE CINTA") {
            assert_eq!(m.bus.ula.border(), 2);
            return;
        }
    }
    panic!("demo.tap no cargó:\n{}", screen_text(&m).join("\n"));
}

/// Reconstruye `demo.tap` como TZX con bloques distintos y comprueba que la ROM lo carga igual.
fn demo_blocks() -> Vec<Vec<u8>> {
    let tap = std::fs::read("tests/data/demo.tap").unwrap();
    zx48::tape::Tape::from_tap(&tap)
        .unwrap()
        .blocks
        .into_iter()
        .map(|b| b.0)
        .collect()
}

fn tzx_file(blocks: Vec<Vec<u8>>) -> Vec<u8> {
    let mut v = b"ZXTape!\x1A\x01\x14".to_vec();
    for b in blocks {
        v.extend(b);
    }
    v
}

fn load_and_expect(tzx: Vec<u8>, needle: &str) {
    let Some(mut m) = booted() else { return };
    m.insert_tape(zx48::tape::load("tzx", &tzx).unwrap());
    m.set_tape_autoplay(true);
    type_load_quotes(&mut m);
    for _ in 0..1500 {
        m.run_frame();
        if screen_text(&m).join("\n").contains(needle) {
            return;
        }
    }
    panic!("no apareció '{needle}':\n{}", screen_text(&m).join("\n"));
}

#[test]
fn rom_loads_tzx_standard_blocks() {
    let b10 = |d: &[u8]| {
        let mut v = vec![0x10, 0xE8, 0x03];
        v.extend((d.len() as u16).to_le_bytes());
        v.extend(d);
        v
    };
    let blocks = demo_blocks().iter().map(|b| b10(b)).collect();
    load_and_expect(tzx_file(blocks), "CARGADO DESDE CINTA");
}

#[test]
fn rom_loads_tzx_turbo_and_pure_blocks() {
    // Cabecera como bloque turbo (0x11) con los tiempos estándar; datos como
    // tono puro (0x12) + pulsos de sincronismo (0x13) + datos puros (0x14).
    let blocks = demo_blocks();
    let mut turbo = vec![0x11];
    for w in [2168u16, 667, 735, 855, 1710, 8063] {
        turbo.extend(w.to_le_bytes());
    }
    turbo.push(8);
    turbo.extend(1000u16.to_le_bytes());
    turbo.extend(&(blocks[0].len() as u32).to_le_bytes()[..3]);
    turbo.extend(&blocks[0]);

    let mut composed = vec![0x12];
    composed.extend(2168u16.to_le_bytes());
    composed.extend(3223u16.to_le_bytes());
    composed.extend([0x13, 2]);
    composed.extend(667u16.to_le_bytes());
    composed.extend(735u16.to_le_bytes());
    composed.push(0x14);
    composed.extend(855u16.to_le_bytes());
    composed.extend(1710u16.to_le_bytes());
    composed.push(8);
    composed.extend(1000u16.to_le_bytes());
    composed.extend(&(blocks[1].len() as u32).to_le_bytes()[..3]);
    composed.extend(&blocks[1]);
    load_and_expect(tzx_file(vec![turbo, composed]), "CARGADO DESDE CINTA");
}
