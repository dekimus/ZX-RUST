//! Carga y validación de la ROM externa (no se redistribuye ninguna ROM).

use crate::error::EmulatorError;
use sha1::{Digest, Sha1};
use std::path::Path;

pub const ROM_SIZE: usize = 16 * 1024;
/// SHA-1 de la ROM estándar 16/48K.
pub const STANDARD_ROM_SHA1: &str = "5ea7c2b824672e914525d1d5c419d71b84a426a2";

pub fn sha1_hex(data: &[u8]) -> String {
    Sha1::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Valida tamaño y, salvo `allow_nonstandard`, el SHA-1. Nunca parchea la ROM.
pub fn validate_rom(
    data: &[u8],
    allow_nonstandard: bool,
) -> Result<Box<[u8; ROM_SIZE]>, EmulatorError> {
    let arr: Box<[u8; ROM_SIZE]> =
        data.to_vec()
            .into_boxed_slice()
            .try_into()
            .map_err(|_| EmulatorError::RomSize {
                expected: ROM_SIZE,
                actual: data.len(),
            })?;
    if !allow_nonstandard {
        let actual_sha1 = sha1_hex(data);
        if actual_sha1 != STANDARD_ROM_SHA1 {
            return Err(EmulatorError::RomChecksum { actual_sha1 });
        }
    }
    Ok(arr)
}

pub fn load_rom(
    path: &Path,
    allow_nonstandard: bool,
) -> Result<Box<[u8; ROM_SIZE]>, EmulatorError> {
    validate_rom(&std::fs::read(path)?, allow_nonstandard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_size() {
        assert!(matches!(
            validate_rom(&[0; 100], true),
            Err(EmulatorError::RomSize { actual: 100, .. })
        ));
    }

    #[test]
    fn nonstandard_rom_needs_flag() {
        let rom = vec![0u8; ROM_SIZE];
        assert!(matches!(
            validate_rom(&rom, false),
            Err(EmulatorError::RomChecksum { .. })
        ));
        assert!(validate_rom(&rom, true).is_ok());
    }

    #[test]
    fn sha1_known_vector() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }
}
