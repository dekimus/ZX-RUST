//! Prefijos y decodificación de campos de opcode (x,y,z,p,q).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefix {
    None,
    Cb,
    Ed,
    Dd,
    Fd,
}

impl Prefix {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0xCB => Some(Self::Cb),
            0xED => Some(Self::Ed),
            0xDD => Some(Self::Dd),
            0xFD => Some(Self::Fd),
            _ => None,
        }
    }

    pub fn code(self) -> u16 {
        match self {
            Self::None => 0x0000,
            Self::Cb => 0xCB00,
            Self::Ed => 0xED00,
            Self::Dd => 0xDD00,
            Self::Fd => 0xFD00,
        }
    }
}

/// Campos estándar de un opcode: `xxyyyzzz`, con `p = y>>1`, `q = y&1`.
#[derive(Clone, Copy, Debug)]
pub struct Fields {
    pub x: u8,
    pub y: u8,
    pub z: u8,
    pub p: u8,
    pub q: u8,
}

impl Fields {
    pub fn new(op: u8) -> Self {
        let y = (op >> 3) & 7;
        Self {
            x: op >> 6,
            y,
            z: op & 7,
            p: y >> 1,
            q: y & 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields() {
        let f = Fields::new(0b01_110_101);
        assert_eq!((f.x, f.y, f.z, f.p, f.q), (1, 6, 5, 3, 0));
    }

    #[test]
    fn prefixes() {
        assert_eq!(Prefix::from_byte(0xCB), Some(Prefix::Cb));
        assert_eq!(Prefix::from_byte(0x00), None);
    }
}
