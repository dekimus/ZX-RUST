//! Layout de la memoria de vídeo y paleta del Spectrum (funciones puras).

/// Dirección del byte de bitmap que contiene el píxel (x, y), con x en 0..256 e y en 0..192.
pub fn bitmap_address(x: u16, y: u16) -> u16 {
    0x4000 | ((y & 0xC0) << 5) | ((y & 0x07) << 8) | ((y & 0x38) << 2) | (x >> 3)
}

/// Dirección del byte de atributo de la celda 8×8 que contiene el píxel (x, y).
pub fn attribute_address(x: u16, y: u16) -> u16 {
    0x5800 + (y >> 3) * 32 + (x >> 3)
}

/// Color base (0..=7: negro, azul, rojo, magenta, verde, cyan, amarillo, blanco) a RGBA.
/// Bit 0 = azul, bit 1 = rojo, bit 2 = verde. BRIGHT usa 0xFF; normal usa 0xCD.
pub fn color_rgba(color: u8, bright: bool) -> [u8; 4] {
    let level = if bright { 0xFF } else { 0xCD };
    let ch = |bit: u8| if color & bit != 0 { level } else { 0 };
    [ch(2), ch(4), ch(1), 0xFF]
}

/// (INK, PAPER) resultantes de un atributo; FLASH los intercambia cuando `flash_inverted`.
pub fn attribute_colors(attr: u8, flash_inverted: bool) -> ([u8; 4], [u8; 4]) {
    let bright = attr & 0x40 != 0;
    let mut ink = attr & 7;
    let mut paper = (attr >> 3) & 7;
    if attr & 0x80 != 0 && flash_inverted {
        std::mem::swap(&mut ink, &mut paper);
    }
    (color_rgba(ink, bright), color_rgba(paper, bright))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_address_boundaries() {
        for (x, y, a) in [
            (0, 0, 0x4000),
            (0, 1, 0x4100),
            (0, 7, 0x4700),
            (0, 8, 0x4020),
            (0, 64, 0x4800),
            (0, 128, 0x5000),
            (248, 191, 0x57FF),
        ] {
            assert_eq!(bitmap_address(x, y), a, "x={x} y={y}");
        }
    }

    #[test]
    fn bitmap_addresses_are_a_bijection_over_bytes() {
        let mut seen = vec![false; 6144];
        for y in 0..192 {
            for xb in 0..32 {
                let a = bitmap_address(xb * 8, y) as usize - 0x4000;
                assert!(!seen[a]);
                seen[a] = true;
            }
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn attribute_addresses() {
        assert_eq!(attribute_address(0, 0), 0x5800);
        assert_eq!(attribute_address(255, 191), 0x5AFF);
        assert_eq!(attribute_address(8, 8), 0x5800 + 33);
    }

    #[test]
    fn palette_and_flash() {
        assert_eq!(color_rgba(2, false), [0xCD, 0, 0, 0xFF]); // rojo
        assert_eq!(color_rgba(4, true), [0, 0xFF, 0, 0xFF]); // verde brillante
        assert_eq!(color_rgba(0, true), [0, 0, 0, 0xFF]); // negro no cambia con BRIGHT
        let attr = 0x80 | (5 << 3) | 2; // FLASH, paper cyan, ink rojo
        let (ink, paper) = attribute_colors(attr, false);
        assert_eq!((ink, paper), (color_rgba(2, false), color_rgba(5, false)));
        let (ink, paper) = attribute_colors(attr, true);
        assert_eq!((ink, paper), (color_rgba(5, false), color_rgba(2, false)));
    }
}
