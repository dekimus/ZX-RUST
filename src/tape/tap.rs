//! Formato `.tap`: secuencia de bloques `u16 LE longitud` + bytes (flag, datos, checksum XOR).

use crate::error::EmulatorError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapeBlock(pub Vec<u8>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapeHeaderKind {
    Program,
    NumberArray,
    CharacterArray,
    Code,
}

/// Cabecera estándar de la ROM (bloque de 19 bytes con flag 0x00).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapeHeader {
    pub kind: TapeHeaderKind,
    /// Nombre de 10 bytes (relleno con espacios).
    pub name: [u8; 10],
    pub length: u16,
    /// PROGRAM: línea de autoarranque (>= 32768 = ninguna). CODE: dirección de inicio.
    pub param1: u16,
    /// PROGRAM: longitud del programa sin variables. CODE: 32768.
    pub param2: u16,
}

impl TapeHeader {
    pub fn name_string(&self) -> String {
        String::from_utf8_lossy(&self.name).trim_end().to_string()
    }

    /// `SCREEN$`: bloque CODE de 6912 bytes en 16384.
    pub fn is_screen(&self) -> bool {
        self.kind == TapeHeaderKind::Code && self.length == 6912 && self.param1 == 16384
    }
}

impl TapeBlock {
    /// Construye un bloque estándar: `flag` + `payload` + checksum XOR.
    pub fn with_checksum(flag: u8, payload: &[u8]) -> Self {
        let mut v = Vec::with_capacity(payload.len() + 2);
        v.push(flag);
        v.extend_from_slice(payload);
        v.push(v.iter().fold(0, |a, b| a ^ b));
        Self(v)
    }

    /// Cabecera estándar con checksum correcto.
    pub fn header(kind: TapeHeaderKind, name: &str, length: u16, param1: u16, param2: u16) -> Self {
        let mut payload = vec![match kind {
            TapeHeaderKind::Program => 0,
            TapeHeaderKind::NumberArray => 1,
            TapeHeaderKind::CharacterArray => 2,
            TapeHeaderKind::Code => 3,
        }];
        let mut n = [b' '; 10];
        for (d, s) in n.iter_mut().zip(name.bytes()) {
            *d = s;
        }
        payload.extend_from_slice(&n);
        for w in [length, param1, param2] {
            payload.extend_from_slice(&w.to_le_bytes());
        }
        Self::with_checksum(0x00, &payload)
    }

    pub fn flag(&self) -> Option<u8> {
        self.0.first().copied()
    }

    /// El XOR de todos los bytes (incluido el checksum) debe ser 0. Un bloque vacío no es válido.
    pub fn checksum_ok(&self) -> bool {
        !self.0.is_empty() && self.0.iter().fold(0, |a, b| a ^ b) == 0
    }

    /// Interpreta el bloque como cabecera estándar (None si no lo es o está corrupto).
    pub fn parse_header(&self) -> Option<TapeHeader> {
        let d = &self.0;
        if d.len() != 19 || d[0] != 0x00 || !self.checksum_ok() {
            return None;
        }
        let kind = match d[1] {
            0 => TapeHeaderKind::Program,
            1 => TapeHeaderKind::NumberArray,
            2 => TapeHeaderKind::CharacterArray,
            3 => TapeHeaderKind::Code,
            _ => return None,
        };
        let w = |i: usize| u16::from_le_bytes([d[i], d[i + 1]]);
        Some(TapeHeader {
            kind,
            name: d[2..12].try_into().ok()?,
            length: w(12),
            param1: w(14),
            param2: w(16),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tape {
    pub blocks: Vec<TapeBlock>,
}

impl Tape {
    pub fn from_blocks(blocks: Vec<TapeBlock>) -> Self {
        Self { blocks }
    }

    /// Parsea un `.tap`. Rechaza longitudes que excedan el fichero; no valida checksums
    /// (un bloque corrupto es un bloque válido para la señal: la ROM dará "Tape loading error").
    pub fn from_tap(data: &[u8]) -> Result<Self, EmulatorError> {
        let mut blocks = Vec::new();
        let mut pos = 0usize;
        while pos < data.len() {
            let Some(len) = data.get(pos..pos + 2) else {
                return Err(EmulatorError::InvalidTape(
                    "longitud de bloque truncada".into(),
                ));
            };
            let len = u16::from_le_bytes([len[0], len[1]]) as usize;
            pos += 2;
            let Some(block) = data.get(pos..pos + len) else {
                return Err(EmulatorError::InvalidTape(format!(
                    "bloque {} truncado: declara {len} bytes, quedan {}",
                    blocks.len(),
                    data.len() - pos
                )));
            };
            blocks.push(TapeBlock(block.to_vec()));
            pos += len;
        }
        Ok(Self { blocks })
    }

    pub fn to_tap(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for b in &self.blocks {
            out.extend_from_slice(&(b.0.len() as u16).to_le_bytes());
            out.extend_from_slice(&b.0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tape {
        Tape::from_blocks(vec![
            TapeBlock::header(TapeHeaderKind::Code, "screen", 6912, 16384, 32768),
            TapeBlock::with_checksum(0xFF, &[1, 2, 3]),
        ])
    }

    #[test]
    fn round_trip() {
        let t = sample();
        assert_eq!(Tape::from_tap(&t.to_tap()).unwrap(), t);
    }

    #[test]
    fn header_fields_and_checksum() {
        let t = sample();
        assert!(t.blocks[0].checksum_ok());
        let h = t.blocks[0].parse_header().unwrap();
        assert_eq!(h.kind, TapeHeaderKind::Code);
        assert_eq!(h.name_string(), "screen");
        assert_eq!((h.length, h.param1, h.param2), (6912, 16384, 32768));
        assert!(h.is_screen());
        assert_eq!(t.blocks[0].0.len(), 19);
        assert!(t.blocks[1].checksum_ok());
        assert_eq!(t.blocks[1].flag(), Some(0xFF));
        assert!(t.blocks[1].parse_header().is_none());
    }

    #[test]
    fn corrupt_checksum_is_detected_but_still_parses() {
        let mut t = sample();
        *t.blocks[1].0.last_mut().unwrap() ^= 1;
        let parsed = Tape::from_tap(&t.to_tap()).unwrap();
        assert!(!parsed.blocks[1].checksum_ok());
        let mut h = sample().blocks[0].clone();
        h.0[3] ^= 0x10;
        assert!(h.parse_header().is_none());
    }

    #[test]
    fn empty_block_and_empty_file() {
        assert!(Tape::from_tap(&[]).unwrap().blocks.is_empty());
        let t = Tape::from_tap(&[0, 0]).unwrap();
        assert_eq!(t.blocks.len(), 1);
        assert!(!t.blocks[0].checksum_ok());
        assert_eq!(t.blocks[0].flag(), None);
    }

    #[test]
    fn truncated_files_are_rejected() {
        assert!(Tape::from_tap(&[5]).is_err());
        assert!(Tape::from_tap(&[5, 0, 1, 2]).is_err());
        let mut ok = sample().to_tap();
        ok.pop();
        assert!(Tape::from_tap(&ok).is_err());
    }
}
