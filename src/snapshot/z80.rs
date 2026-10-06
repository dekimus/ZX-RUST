//! `.z80` (v1, v2, v3) para hardware 48K. Rechaza explícitamente 16K, 128K, Interface 1,
//! MGT, SamRam, etc. en lugar de cargar parcialmente.
//!
//! Notas de interpretación:
//! - Cabecera base de 30 bytes; si PC (bytes 6-7) != 0 es v1 (RAM de 48K, posiblemente comprimida).
//! - v2/v3: cabecera adicional (23 / 54 / 55 bytes) y bloques de 16 KiB con longitud, página y datos.
//!   Páginas 48K: 4 = 0x8000-0xBFFF, 5 = 0xC000-0xFFFF, 8 = 0x4000-0x7FFF; la 0 (ROM) solo se
//!   acepta si coincide con la ROM de la máquina.
//! - Compresión: `ED ED nn bb` = `nn` copias de `bb`; un `ED` suelto va seguido de un byte literal.
//! - v3: los contadores de T-states (bytes 55-57) restauran la fase exacta del frame
//!   (`((hi+1)%4)*q + (q-lo-1)` con `q = 69888/4`, según libspectrum).
//! - Los flags "Issue 2", "doble frecuencia de interrupción" y joystick (byte 29) se ignoran.

use super::{Snapshot, le16};
use crate::error::EmulatorError;
use crate::machine::memory::RAM_SIZE;
use crate::ula::timing::TSTATES_PER_FRAME;

const BASE_HEADER: usize = 30;
const PAGE_SIZE: usize = 16 * 1024;
const QUARTER: u32 = TSTATES_PER_FRAME / 4;

fn invalid<T>(m: impl Into<String>) -> Result<T, EmulatorError> {
    Err(EmulatorError::InvalidSnapshot(m.into()))
}

/// Descomprime `ED ED nn bb`. Debe producir exactamente `expected` bytes.
pub fn decompress(data: &[u8], expected: usize) -> Result<Vec<u8>, EmulatorError> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0xED && data.get(i + 1) == Some(&0xED) {
            let (Some(&count), Some(&value)) = (data.get(i + 2), data.get(i + 3)) else {
                return invalid("secuencia ED ED truncada");
            };
            if count == 0 {
                return invalid("secuencia ED ED con contador 0");
            }
            if out.len() + count as usize > expected {
                return invalid("datos descomprimidos exceden el tamaño esperado");
            }
            out.extend(std::iter::repeat_n(value, count as usize));
            i += 4;
        } else {
            if out.len() >= expected {
                return invalid("datos descomprimidos exceden el tamaño esperado");
            }
            out.push(data[i]);
            i += 1;
        }
    }
    if out.len() != expected {
        return invalid(format!(
            "datos descomprimidos: {} bytes, se esperaban {expected}",
            out.len()
        ));
    }
    Ok(out)
}

/// Comprime con `ED ED nn bb` (corridas >= 5, y toda corrida de ED >= 2); tras un `ED` suelto
/// el byte siguiente se emite literal.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        let mut run = 1;
        while i + run < data.len() && data[i + run] == b && run < 255 {
            run += 1;
        }
        if run >= 5 || (b == 0xED && run >= 2) {
            out.extend_from_slice(&[0xED, 0xED, run as u8, b]);
            i += run;
        } else if b == 0xED {
            out.push(0xED);
            if let Some(&next) = data.get(i + 1) {
                out.push(next);
            }
            i += 2;
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

pub fn parse(data: &[u8], machine_rom: Option<&[u8]>) -> Result<Snapshot, EmulatorError> {
    if data.len() < BASE_HEADER {
        return invalid("cabecera .z80 truncada");
    }
    let mut s = Snapshot::blank();
    let h = data;
    let b12 = if h[12] == 255 { 1 } else { h[12] };
    if b12 & 0x10 != 0 {
        return Err(EmulatorError::UnsupportedHardware("SamRam ROM".into()));
    }
    let im = h[29] & 3;
    if im > 2 {
        return invalid(format!("modo de interrupción {im}"));
    }
    let r = &mut s.regs;
    r.a = h[0];
    r.f = h[1];
    r.set_bc(le16(h, 2));
    r.set_hl(le16(h, 4));
    r.sp = le16(h, 8);
    r.i = h[10];
    r.r = (h[11] & 0x7F) | ((b12 & 1) << 7);
    r.set_de(le16(h, 13));
    r.set_bc_alt(le16(h, 15));
    r.set_de_alt(le16(h, 17));
    r.set_hl_alt(le16(h, 19));
    r.a_ = h[21];
    r.f_ = h[22];
    r.iy = le16(h, 23);
    r.ix = le16(h, 25);
    s.iff1 = h[27] != 0;
    s.iff2 = h[28] != 0;
    s.im = im;
    s.border = (b12 >> 1) & 7;

    let pc_v1 = le16(h, 6);
    if pc_v1 != 0 {
        s.regs.pc = pc_v1;
        let mut body = &data[BASE_HEADER..];
        if b12 & 0x20 != 0 {
            if body.ends_with(&[0x00, 0xED, 0xED, 0x00]) {
                body = &body[..body.len() - 4];
            }
            s.ram.copy_from_slice(&decompress(body, RAM_SIZE)?);
        } else if body.len() == RAM_SIZE {
            s.ram.copy_from_slice(body);
        } else {
            return invalid(format!(
                "RAM v1 sin comprimir: {} bytes, se esperaban {RAM_SIZE}",
                body.len()
            ));
        }
        return Ok(s);
    }

    if data.len() < BASE_HEADER + 2 {
        return invalid("falta la longitud de la cabecera adicional");
    }
    let ext_len = le16(data, BASE_HEADER) as usize;
    if !matches!(ext_len, 23 | 54 | 55) {
        return invalid(format!(
            "longitud de cabecera adicional desconocida: {ext_len}"
        ));
    }
    let blocks_at = BASE_HEADER + 2 + ext_len;
    if data.len() < blocks_at {
        return invalid("cabecera adicional truncada");
    }
    let ext = &data[BASE_HEADER + 2..blocks_at];
    s.regs.pc = le16(ext, 0);
    let hw = ext[2];
    if hw != 0 {
        return Err(EmulatorError::UnsupportedHardware(format!(
            "modo de hardware {hw} (solo 48K puro, modo 0, está soportado)"
        )));
    }
    if ext[5] & 0x80 != 0 {
        return Err(EmulatorError::UnsupportedHardware(
            "Spectrum 16K (bit 7 de 'modificar hardware')".into(),
        ));
    }
    if ext_len >= 54 {
        let (lo, hi) = (le16(ext, 23) as u32, ext[25] as u32);
        if lo >= QUARTER || hi > 3 {
            return invalid(format!(
                "contadores de T-states fuera de rango (lo={lo}, hi={hi})"
            ));
        }
        s.frame_tstate = Some(((hi + 1) % 4) * QUARTER + (QUARTER - lo - 1));
    }

    let mut pos = blocks_at;
    let mut seen = [false; 3]; // páginas 8, 4, 5
    while pos < data.len() {
        let Some(bh) = data.get(pos..pos + 3) else {
            return invalid("cabecera de bloque de memoria truncada");
        };
        let (clen, page) = (u16::from_le_bytes([bh[0], bh[1]]), bh[2]);
        pos += 3;
        let raw_len = if clen == 0xFFFF {
            PAGE_SIZE
        } else {
            clen as usize
        };
        let Some(raw) = data.get(pos..pos + raw_len) else {
            return invalid(format!("bloque de la página {page} truncado"));
        };
        pos += raw_len;
        let page_data = if clen == 0xFFFF {
            raw.to_vec()
        } else {
            decompress(raw, PAGE_SIZE)?
        };
        let (slot, ram_off) = match page {
            8 => (0, 0),
            4 => (1, PAGE_SIZE),
            5 => (2, 2 * PAGE_SIZE),
            0 => {
                match machine_rom {
                    Some(rom) if rom == &page_data[..] => {}
                    _ => {
                        return Err(EmulatorError::UnsupportedSnapshot(
                            "el snapshot incluye una ROM distinta de la de la máquina".into(),
                        ));
                    }
                }
                continue;
            }
            p => {
                return Err(EmulatorError::UnsupportedHardware(format!(
                    "página de memoria {p} no existe en el 48K"
                )));
            }
        };
        if std::mem::replace(&mut seen[slot], true) {
            return invalid(format!("página {page} duplicada"));
        }
        s.ram[ram_off..ram_off + PAGE_SIZE].copy_from_slice(&page_data);
    }
    if let Some(i) = seen.iter().position(|&x| !x) {
        return invalid(format!("falta la página {}", [8, 4, 5][i]));
    }
    Ok(s)
}

/// Escribe un `.z80` v3 (48K) con RAM comprimida y la fase del frame si se conoce.
pub fn build(s: &Snapshot) -> Vec<u8> {
    let r = &s.regs;
    let mut out = Vec::new();
    out.push(r.a);
    out.push(r.f);
    out.extend_from_slice(&r.bc().to_le_bytes());
    out.extend_from_slice(&r.hl().to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // PC = 0 => v2/v3
    out.extend_from_slice(&r.sp.to_le_bytes());
    out.push(r.i);
    out.push(r.r & 0x7F);
    out.push((r.r >> 7) | ((s.border & 7) << 1));
    out.extend_from_slice(&r.de().to_le_bytes());
    out.extend_from_slice(&r.bc_alt().to_le_bytes());
    out.extend_from_slice(&r.de_alt().to_le_bytes());
    out.extend_from_slice(&r.hl_alt().to_le_bytes());
    out.push(r.a_);
    out.push(r.f_);
    out.extend_from_slice(&r.iy.to_le_bytes());
    out.extend_from_slice(&r.ix.to_le_bytes());
    out.push(s.iff1 as u8);
    out.push(s.iff2 as u8);
    out.push(s.im & 3);
    debug_assert_eq!(out.len(), BASE_HEADER);

    out.extend_from_slice(&54u16.to_le_bytes());
    let mut ext = [0u8; 54];
    ext[0..2].copy_from_slice(&r.pc.to_le_bytes());
    // ext[2] = 0: 48K. El resto (sonido, IF1, MGT, ...) a 0.
    if let Some(t) = s.frame_tstate {
        let t = t % TSTATES_PER_FRAME;
        let hi = (t / QUARTER + 3) % 4;
        let lo = QUARTER - 1 - t % QUARTER;
        ext[23..25].copy_from_slice(&(lo as u16).to_le_bytes());
        ext[25] = hi as u8;
    }
    out.extend_from_slice(&ext);

    for (page, off) in [(8u8, 0usize), (4, PAGE_SIZE), (5, 2 * PAGE_SIZE)] {
        let data = &s.ram[off..off + PAGE_SIZE];
        let c = compress(data);
        if c.len() < PAGE_SIZE {
            out.extend_from_slice(&(c.len() as u16).to_le_bytes());
            out.push(page);
            out.extend_from_slice(&c);
        } else {
            out.extend_from_slice(&0xFFFFu16.to_le_bytes());
            out.push(page);
            out.extend_from_slice(data);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::Registers;

    fn sample() -> Snapshot {
        let mut s = Snapshot::blank();
        s.regs = Registers {
            a: 1,
            f: 2,
            b: 3,
            c: 4,
            d: 5,
            e: 6,
            h: 7,
            l: 8,
            a_: 9,
            f_: 10,
            b_: 11,
            c_: 12,
            d_: 13,
            e_: 14,
            h_: 15,
            l_: 16,
            ix: 0x1234,
            iy: 0x5678,
            sp: 0xFF00,
            pc: 0x8123,
            i: 0x3F,
            r: 0xA5,
        };
        s.iff1 = true;
        s.iff2 = false;
        s.im = 1;
        s.border = 6;
        s.frame_tstate = Some(40_000);
        // Mezcla de datos comprimibles, aleatorios y trampas con ED.
        let mut x = 12345u32;
        for (i, b) in s.ram.iter_mut().enumerate() {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = match (i / 1000) % 4 {
                0 => 0,
                1 => (x >> 24) as u8,
                2 => 0xED,
                _ => {
                    if i % 3 == 0 {
                        0xED
                    } else {
                        (x >> 24) as u8
                    }
                }
            };
        }
        s
    }

    #[test]
    fn compression_round_trip_including_ed_edge_cases() {
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0xED],
            vec![0xED, 0x00],
            vec![0xED, 0xED],
            vec![0xED, 5, 5, 5, 5, 5, 5],
            vec![1, 1, 1, 1, 1],
            vec![1, 1, 1, 1],
            vec![0xED; 300],
            vec![7; 600],
            vec![0, 0xED, 0xED, 0xED, 0, 0xED],
        ];
        for c in cases {
            assert_eq!(decompress(&compress(&c), c.len()).unwrap(), c, "{c:02X?}");
        }
    }

    #[test]
    fn decompression_rejects_malformed_data() {
        assert!(decompress(&[0xED, 0xED, 5], 5).is_err()); // truncada
        assert!(decompress(&[0xED, 0xED, 0, 1], 0).is_err()); // contador 0
        assert!(decompress(&[0xED, 0xED, 10, 1], 5).is_err()); // excede
        assert!(decompress(&[1, 2, 3], 4).is_err()); // corta
        assert!(decompress(&[1, 2, 3, 4, 5], 4).is_err()); // larga
    }

    #[test]
    fn v3_round_trip_restores_everything() {
        let s = sample();
        let bytes = build(&s);
        let p = parse(&bytes, None).unwrap();
        assert_eq!(p, s);
    }

    #[test]
    fn frame_tstate_round_trips_at_quarter_boundaries() {
        for t in [0u32, 1, 17471, 17472, 17473, 34943, 34944, 69887, 14335] {
            let mut s = Snapshot::blank();
            s.frame_tstate = Some(t);
            assert_eq!(
                parse(&build(&s), None).unwrap().frame_tstate,
                Some(t),
                "t={t}"
            );
        }
    }

    #[test]
    fn r_register_bit7_and_border_encoding() {
        let mut s = Snapshot::blank();
        s.regs.r = 0x80 | 0x15;
        s.border = 5;
        let b = build(&s);
        assert_eq!(b[11], 0x15);
        assert_eq!(b[12], 1 | (5 << 1));
    }

    fn v1_header(compressed: bool) -> Vec<u8> {
        let mut h = vec![0u8; 30];
        h[6..8].copy_from_slice(&0x9000u16.to_le_bytes()); // PC != 0 => v1
        h[8..10].copy_from_slice(&0xFF00u16.to_le_bytes());
        h[12] = (3 << 1) | if compressed { 0x20 } else { 0 };
        h[27] = 1;
        h[28] = 1;
        h
    }

    #[test]
    fn v1_uncompressed_and_compressed() {
        let ram: Vec<u8> = (0..RAM_SIZE)
            .map(|i| if i < 20000 { 0 } else { (i % 251) as u8 })
            .collect();
        let mut raw = v1_header(false);
        raw.extend_from_slice(&ram);
        let p = parse(&raw, None).unwrap();
        assert_eq!((p.regs.pc, p.border, p.iff1), (0x9000, 3, true));
        assert_eq!(&p.ram[..], &ram[..]);

        let mut comp = v1_header(true);
        comp.extend_from_slice(&compress(&ram));
        comp.extend_from_slice(&[0, 0xED, 0xED, 0]); // marcador final
        assert_eq!(parse(&comp, None).unwrap().ram[..], ram[..]);
        // Sin marcador también es aceptable.
        let mut nomark = v1_header(true);
        nomark.extend_from_slice(&compress(&ram));
        assert_eq!(parse(&nomark, None).unwrap().ram[..], ram[..]);
    }

    #[test]
    fn v1_wrong_size_rejected() {
        let mut raw = v1_header(false);
        raw.extend_from_slice(&[0; 1000]);
        assert!(parse(&raw, None).is_err());
    }

    #[test]
    fn rejects_unsupported_hardware_instead_of_partial_load() {
        let good = build(&sample());
        for hw in [1u8, 2, 3, 4, 7, 9] {
            let mut b = good.clone();
            b[34] = hw;
            assert!(
                matches!(parse(&b, None), Err(EmulatorError::UnsupportedHardware(_))),
                "hw={hw}"
            );
        }
        let mut b = good.clone();
        b[37] |= 0x80; // 16K
        assert!(matches!(
            parse(&b, None),
            Err(EmulatorError::UnsupportedHardware(_))
        ));
        let mut b = good.clone();
        b[12] |= 0x10; // SamRam
        assert!(matches!(
            parse(&b, None),
            Err(EmulatorError::UnsupportedHardware(_))
        ));
        // Página de 128K en un snapshot declarado 48K.
        let mut b = good;
        let first_block_page = 32 + 54 + 2;
        b[first_block_page] = 3;
        assert!(parse(&b, None).is_err());
    }

    #[test]
    fn rejects_malformed_blocks() {
        let good = build(&sample());
        // truncado
        assert!(parse(&good[..good.len() - 10], None).is_err());
        assert!(parse(&good[..20], None).is_err());
        // longitud de cabecera adicional desconocida
        let mut b = good.clone();
        b[30] = 40;
        assert!(parse(&b, None).is_err());
        // falta una página
        let first_len = u16::from_le_bytes([good[86], good[87]]) as usize;
        let cut = 86
            + 3
            + if first_len == 0xFFFF {
                PAGE_SIZE
            } else {
                first_len
            };
        assert!(parse(&good[..cut], None).is_err());
        // página duplicada
        let mut dup = good.clone();
        dup.extend_from_slice(&good[86..cut]);
        assert!(parse(&dup, None).is_err());
        // IM inválido
        let mut b = good;
        b[29] = 3;
        assert!(parse(&b, None).is_err());
    }

    #[test]
    fn rom_page_must_match_machine_rom() {
        let mut b = build(&sample());
        let rom = vec![0xAAu8; PAGE_SIZE];
        b.extend_from_slice(&0xFFFFu16.to_le_bytes());
        b.push(0);
        b.extend_from_slice(&rom);
        assert!(parse(&b, Some(&rom)).is_ok());
        assert!(matches!(
            parse(&b, Some(&vec![0u8; PAGE_SIZE])),
            Err(EmulatorError::UnsupportedSnapshot(_))
        ));
        assert!(matches!(
            parse(&b, None),
            Err(EmulatorError::UnsupportedSnapshot(_))
        ));
    }
}
