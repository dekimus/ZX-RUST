//! Snapshots: parsers y escritores independientes de la máquina. Producen/consumen un
//! `Snapshot` neutral; `Spectrum48` lo aplica con `load_snapshot`/`snapshot`.

pub mod scr;
pub mod sna;
pub mod z80;

use crate::cpu::Registers;
use crate::machine::memory::RAM_SIZE;

/// Estado de una máquina 48K: lo que contienen los formatos SNA/Z80 en 48K.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// `pc` es el valor real de continuación (SNA lo extrae de la pila).
    pub regs: Registers,
    pub iff1: bool,
    pub iff2: bool,
    pub im: u8,
    pub border: u8,
    /// RAM 0x4000..=0xFFFF.
    pub ram: Box<[u8; RAM_SIZE]>,
    /// T-state dentro del frame si el formato lo guarda (Z80 v3); si no, se arranca en 0.
    pub frame_tstate: Option<u32>,
}

impl Snapshot {
    pub fn blank() -> Self {
        Self {
            regs: Registers::default(),
            iff1: false,
            iff2: false,
            im: 0,
            border: 7,
            ram: Box::new([0; RAM_SIZE]),
            frame_tstate: None,
        }
    }
}

pub(crate) fn le16(d: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([d[i], d[i + 1]])
}
