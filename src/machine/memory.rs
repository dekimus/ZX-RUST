//! Mapa de memoria 48K: ROM 0x0000..=0x3FFF, RAM 0x4000..=0xFFFF.

use crate::rom::loader::ROM_SIZE;

pub const RAM_START: u16 = 0x4000;
pub const RAM_SIZE: usize = 48 * 1024;

pub struct Memory48 {
    rom: Box<[u8; ROM_SIZE]>,
    ram: Box<[u8; RAM_SIZE]>,
}

impl Memory48 {
    /// RAM a cero (determinista; la aleatorización de power-on sería opcional y aparte).
    pub fn new(rom: Box<[u8; ROM_SIZE]>) -> Self {
        Self {
            rom,
            ram: Box::new([0; RAM_SIZE]),
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        if addr < RAM_START {
            self.rom[addr as usize]
        } else {
            self.ram[(addr - RAM_START) as usize]
        }
    }

    /// Las escrituras a ROM se ignoran.
    pub fn write(&mut self, addr: u16, value: u8) {
        if addr >= RAM_START {
            self.ram[(addr - RAM_START) as usize] = value;
        }
    }

    pub fn rom(&self) -> &[u8; ROM_SIZE] {
        &self.rom
    }

    pub fn load_ram(&mut self, data: &[u8; RAM_SIZE]) {
        self.ram.copy_from_slice(data);
    }

    pub fn ram(&self) -> &[u8; RAM_SIZE] {
        &self.ram
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Memory48 {
        let mut rom = Box::new([0u8; ROM_SIZE]);
        rom[0] = 0xF3;
        rom[0x3FFF] = 0xAA;
        Memory48::new(rom)
    }

    #[test]
    fn rom_reads() {
        let m = mem();
        assert_eq!(m.read(0x0000), 0xF3);
        assert_eq!(m.read(0x3FFF), 0xAA);
    }

    #[test]
    fn rom_write_ignored() {
        let mut m = mem();
        m.write(0x0000, 0x55);
        m.write(0x3FFF, 0x55);
        assert_eq!(m.read(0x0000), 0xF3);
        assert_eq!(m.read(0x3FFF), 0xAA);
    }

    #[test]
    fn ram_boundaries() {
        let mut m = mem();
        m.write(0x4000, 0x12);
        m.write(0xFFFF, 0x34);
        assert_eq!(m.read(0x4000), 0x12);
        assert_eq!(m.read(0xFFFF), 0x34);
        assert_eq!(m.ram()[0], 0x12);
        assert_eq!(m.ram()[RAM_SIZE - 1], 0x34);
    }
}
