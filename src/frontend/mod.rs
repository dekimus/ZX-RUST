//! Frontend gráfico (egui/eframe). Capa de presentación y control: no contiene lógica de CPU,
//! ULA, memoria, teclado, cinta ni temporización; solo usa la API pública del core.

pub mod actions;
pub mod app;
pub mod audio;
pub mod icon;
pub mod keymap;
pub mod settings;
pub mod viewport;
