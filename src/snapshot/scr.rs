//! `.scr`: volcado de 6912 bytes de 0x4000..0x5AFF.

use crate::error::EmulatorError;

pub const SCR_SIZE: usize = 6912;

/// Valida un `.scr` estándar (6144 bitmap + 768 atributos).
pub fn parse(data: &[u8]) -> Result<&[u8; SCR_SIZE], EmulatorError> {
    data.try_into().map_err(|_| {
        EmulatorError::InvalidSnapshot(format!(
            ".scr debe tener {SCR_SIZE} bytes, tiene {}",
            data.len()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_is_validated() {
        assert!(parse(&[0; SCR_SIZE]).is_ok());
        assert!(parse(&[0; SCR_SIZE - 1]).is_err());
        assert!(parse(&[0; 12288]).is_err()); // Timex hi-res: no soportado
    }
}
