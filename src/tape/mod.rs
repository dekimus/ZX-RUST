//! Cinta: parseo de formatos (independiente de la emulación) y generación de señal EAR.

pub mod signal;
pub mod tap;
pub mod tzx;

use crate::error::EmulatorError;

pub use signal::{Playable, SignalBlock, TapePlayer};
pub use tap::{Tape, TapeBlock, TapeHeader, TapeHeaderKind};
pub use tzx::Tzx;

/// Carga una cinta según la extensión del fichero (`tap`, `tzx`).
pub fn load(extension: &str, data: &[u8]) -> Result<Playable, EmulatorError> {
    match extension.to_ascii_lowercase().as_str() {
        "tap" => Ok(Tape::from_tap(data)?.into()),
        "tzx" => Ok(Tzx::parse(data)?.into()),
        e => Err(EmulatorError::InvalidTape(format!(
            "formato de cinta desconocido: .{e}"
        ))),
    }
}
