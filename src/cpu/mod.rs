//! Z80 independiente de la máquina: solo conoce el trait `Bus`.

pub mod disasm;
pub mod flags;
pub mod opcode;
pub mod registers;
pub mod timing;
pub mod z80;

pub use registers::Registers;
pub use z80::Z80;
