#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registers {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub a_: u8,
    pub f_: u8,
    pub b_: u8,
    pub c_: u8,
    pub d_: u8,
    pub e_: u8,
    pub h_: u8,
    pub l_: u8,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,
    pub i: u8,
    pub r: u8,
}

fn join(hi: u8, lo: u8) -> u16 {
    (hi as u16) << 8 | lo as u16
}

impl Registers {
    pub fn af(&self) -> u16 {
        join(self.a, self.f)
    }
    pub fn bc(&self) -> u16 {
        join(self.b, self.c)
    }
    pub fn de(&self) -> u16 {
        join(self.d, self.e)
    }
    pub fn hl(&self) -> u16 {
        join(self.h, self.l)
    }
    pub fn af_alt(&self) -> u16 {
        join(self.a_, self.f_)
    }
    pub fn bc_alt(&self) -> u16 {
        join(self.b_, self.c_)
    }
    pub fn de_alt(&self) -> u16 {
        join(self.d_, self.e_)
    }
    pub fn hl_alt(&self) -> u16 {
        join(self.h_, self.l_)
    }
    pub fn set_af_alt(&mut self, v: u16) {
        self.a_ = (v >> 8) as u8;
        self.f_ = v as u8;
    }
    pub fn set_bc_alt(&mut self, v: u16) {
        self.b_ = (v >> 8) as u8;
        self.c_ = v as u8;
    }
    pub fn set_de_alt(&mut self, v: u16) {
        self.d_ = (v >> 8) as u8;
        self.e_ = v as u8;
    }
    pub fn set_hl_alt(&mut self, v: u16) {
        self.h_ = (v >> 8) as u8;
        self.l_ = v as u8;
    }
    pub fn set_af(&mut self, v: u16) {
        self.a = (v >> 8) as u8;
        self.f = v as u8;
    }
    pub fn set_bc(&mut self, v: u16) {
        self.b = (v >> 8) as u8;
        self.c = v as u8;
    }
    pub fn set_de(&mut self, v: u16) {
        self.d = (v >> 8) as u8;
        self.e = v as u8;
    }
    pub fn set_hl(&mut self, v: u16) {
        self.h = (v >> 8) as u8;
        self.l = v as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs() {
        let mut r = Registers::default();
        r.set_bc(0x1234);
        assert_eq!((r.b, r.c), (0x12, 0x34));
        r.set_hl(0xABCD);
        assert_eq!(r.hl(), 0xABCD);
        r.set_af(0xFF01);
        assert_eq!((r.a, r.f), (0xFF, 0x01));
        r.set_de(0x0102);
        assert_eq!(r.de(), 0x0102);
    }
}
