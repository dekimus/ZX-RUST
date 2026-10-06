//! `.sna` 48K: cabecera de 27 bytes + 49152 bytes de RAM. El PC no está en la cabecera:
//! se guardó en la pila (instantánea tomada en una NMI); cargar equivale a ejecutar `RETN`.

use super::{Snapshot, le16};
use crate::error::EmulatorError;
use crate::machine::memory::{RAM_SIZE, RAM_START};

const HEADER: usize = 27;

pub fn parse(data: &[u8]) -> Result<Snapshot, EmulatorError> {
    if data.len() != HEADER + RAM_SIZE {
        return Err(if matches!(data.len(), 131_103 | 147_487) {
            EmulatorError::UnsupportedHardware(".sna de 128K".into())
        } else {
            EmulatorError::InvalidSnapshot(format!(
                ".sna 48K debe tener {} bytes, tiene {}",
                HEADER + RAM_SIZE,
                data.len()
            ))
        });
    }
    let im = data[25];
    if im > 2 {
        return Err(EmulatorError::InvalidSnapshot(format!(
            "modo de interrupción {im}"
        )));
    }
    let border = data[26];
    if border > 7 {
        return Err(EmulatorError::InvalidSnapshot(format!(
            "color de border {border}"
        )));
    }
    let mut s = Snapshot::blank();
    s.ram.copy_from_slice(&data[HEADER..]);
    let r = &mut s.regs;
    r.i = data[0];
    r.set_hl_alt(le16(data, 1));
    r.set_de_alt(le16(data, 3));
    r.set_bc_alt(le16(data, 5));
    r.set_af_alt(le16(data, 7));
    r.set_hl(le16(data, 9));
    r.set_de(le16(data, 11));
    r.set_bc(le16(data, 13));
    r.iy = le16(data, 15);
    r.ix = le16(data, 17);
    r.r = data[20];
    r.set_af(le16(data, 21));
    r.sp = le16(data, 23);
    s.iff2 = data[19] & 0x04 != 0;
    s.iff1 = s.iff2; // RETN restaura IFF1 desde IFF2
    s.im = im;
    s.border = border;
    // Equivale a RETN: PC = pop().
    let sp = s.regs.sp;
    if !(RAM_START..0xFFFF).contains(&sp) {
        return Err(EmulatorError::InvalidSnapshot(format!(
            "SP={sp:#06X} no permite extraer PC de la RAM"
        )));
    }
    let i = (sp - RAM_START) as usize;
    s.regs.pc = u16::from_le_bytes([s.ram[i], s.ram[i + 1]]);
    s.regs.sp = sp.wrapping_add(2);
    Ok(s)
}

/// Escribe un `.sna`. Como el formato no tiene campo PC, se empuja a la pila (en la copia de RAM
/// que se guarda). Solo se guarda IFF2. Requiere que la pila esté en RAM.
pub fn build(s: &Snapshot) -> Result<Vec<u8>, EmulatorError> {
    let sp = s.regs.sp;
    if sp < RAM_START + 2 {
        return Err(EmulatorError::UnsupportedSnapshot(format!(
            "SP={sp:#06X}: no se puede apilar PC en RAM para generar un .sna"
        )));
    }
    let new_sp = sp - 2;
    let mut ram = s.ram.clone();
    let i = (new_sp - RAM_START) as usize;
    ram[i..i + 2].copy_from_slice(&s.regs.pc.to_le_bytes());
    let r = &s.regs;
    let mut out = Vec::with_capacity(HEADER + RAM_SIZE);
    out.push(r.i);
    for w in [
        r.hl_alt(),
        r.de_alt(),
        r.bc_alt(),
        r.af_alt(),
        r.hl(),
        r.de(),
        r.bc(),
        r.iy,
        r.ix,
    ] {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out.push(if s.iff2 { 0x04 } else { 0 });
    out.push(r.r);
    out.extend_from_slice(&r.af().to_le_bytes());
    out.extend_from_slice(&new_sp.to_le_bytes());
    out.push(s.im);
    out.push(s.border);
    out.extend_from_slice(&ram[..]);
    Ok(out)
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
        s.iff2 = true;
        s.im = 2;
        s.border = 5;
        for (i, b) in s.ram.iter_mut().enumerate() {
            *b = (i * 7 + i / 256) as u8;
        }
        s
    }

    #[test]
    fn build_then_parse_restores_state_and_stack_semantics() {
        let s = sample();
        let bytes = build(&s).unwrap();
        assert_eq!(bytes.len(), 49179);
        // SP guardado = SP real - 2; PC en la pila.
        assert_eq!(le16(&bytes, 23), 0xFEFE);
        let p = parse(&bytes).unwrap();
        assert_eq!(p.regs, s.regs);
        assert_eq!((p.iff1, p.iff2, p.im, p.border), (true, true, 2, 5));
        // La RAM solo difiere en los 2 bytes donde se apiló PC.
        let diff: Vec<_> = (0..RAM_SIZE).filter(|&i| p.ram[i] != s.ram[i]).collect();
        assert!(diff.len() <= 2);
        assert!(
            diff.iter()
                .all(|&i| i + 0x4000 == 0xFEFE || i + 0x4000 == 0xFEFF)
        );
        // Idempotente.
        assert_eq!(build(&p).unwrap(), bytes);
    }

    #[test]
    fn header_layout_matches_format() {
        let bytes = build(&sample()).unwrap();
        assert_eq!(bytes[0], 0x3F); // I
        assert_eq!(le16(&bytes, 1), 0x0F10); // HL'
        assert_eq!(le16(&bytes, 7), 0x090A); // AF'
        assert_eq!(le16(&bytes, 9), 0x0708); // HL
        assert_eq!(le16(&bytes, 15), 0x5678); // IY
        assert_eq!(le16(&bytes, 17), 0x1234); // IX
        assert_eq!(bytes[19], 0x04); // IFF2
        assert_eq!(bytes[20], 0xA5); // R
        assert_eq!(le16(&bytes, 21), 0x0102); // AF
        assert_eq!((bytes[25], bytes[26]), (2, 5));
    }

    #[test]
    fn iff2_clear_gives_di_after_retn() {
        let mut s = sample();
        s.iff1 = false;
        s.iff2 = false;
        let p = parse(&build(&s).unwrap()).unwrap();
        assert!(!p.iff1 && !p.iff2);
    }

    #[test]
    fn rejects_bad_files() {
        let good = build(&sample()).unwrap();
        assert!(parse(&good[..good.len() - 1]).is_err());
        assert!(parse(&[0; 10]).is_err());
        let mut bad = good.clone();
        bad[25] = 3;
        assert!(parse(&bad).is_err());
        let mut bad = good.clone();
        bad[26] = 8;
        assert!(parse(&bad).is_err());
        let mut bad = good.clone();
        bad[23..25].copy_from_slice(&0x1000u16.to_le_bytes()); // SP en ROM
        assert!(parse(&bad).is_err());
        let mut bad = good;
        bad[23..25].copy_from_slice(&0xFFFFu16.to_le_bytes());
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn detects_128k_sna() {
        let big = vec![0u8; 131103];
        assert!(matches!(
            parse(&big),
            Err(EmulatorError::UnsupportedHardware(_))
        ));
    }

    #[test]
    fn build_rejects_stack_in_rom() {
        let mut s = sample();
        s.regs.sp = 0x4001;
        assert!(matches!(
            build(&s),
            Err(EmulatorError::UnsupportedSnapshot(_))
        ));
    }
}
