//! Núcleo del emulador de ZX Spectrum 48K. Sin dependencias de frontend.

pub mod audio;
pub mod cpu;
pub mod debug;
pub mod error;
#[cfg(feature = "gui")]
pub mod frontend;
pub mod input;
pub mod machine;
pub mod rom;
pub mod snapshot;
pub mod tape;
pub mod ula;

pub use error::EmulatorError;
pub use input::kempston::JoyButton;
pub use input::keyboard::SpectrumKey;
pub use machine::spectrum48::Spectrum48;
