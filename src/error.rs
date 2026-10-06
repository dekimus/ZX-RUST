use std::fmt;

#[derive(Debug)]
pub enum EmulatorError {
    /// La ROM no tiene exactamente 16384 bytes.
    RomSize {
        expected: usize,
        actual: usize,
    },
    /// SHA-1 de la ROM distinto del estándar (puede permitirse explícitamente).
    RomChecksum {
        actual_sha1: String,
    },
    Io(std::io::Error),
    InvalidTape(String),
    UnsupportedTape(String),
    InvalidSnapshot(String),
    UnsupportedSnapshot(String),
    UnsupportedHardware(String),
}

impl fmt::Display for EmulatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RomSize { expected, actual } => {
                write!(
                    f,
                    "tamaño de ROM inválido: esperado {expected}, obtenido {actual}"
                )
            }
            Self::RomChecksum { actual_sha1 } => {
                write!(f, "SHA-1 de ROM no estándar: {actual_sha1}")
            }
            Self::InvalidSnapshot(m) => write!(f, "snapshot inválido: {m}"),
            Self::UnsupportedSnapshot(m) => write!(f, "snapshot no soportado: {m}"),
            Self::UnsupportedHardware(m) => write!(f, "hardware no soportado: {m}"),
            Self::UnsupportedTape(m) => write!(f, "cinta no soportada: {m}"),
            Self::InvalidTape(m) => write!(f, "cinta inválida: {m}"),
            Self::Io(e) => write!(f, "error de E/S: {e}"),
        }
    }
}

impl std::error::Error for EmulatorError {}

impl From<std::io::Error> for EmulatorError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
