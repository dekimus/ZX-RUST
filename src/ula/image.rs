//! Exportación del framebuffer RGBA a formatos de imagen simples (sin dependencias).

use super::{FB_HEIGHT, FB_WIDTH};

/// PPM binario (P6).
pub fn to_ppm(rgba: &[u8]) -> Vec<u8> {
    let mut out = format!("P6\n{FB_WIDTH} {FB_HEIGHT}\n255\n").into_bytes();
    out.extend(rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
    out
}

/// BMP de 24 bits sin compresión (filas de abajo arriba, en BGR, relleno a 4 bytes).
pub fn to_bmp(rgba: &[u8]) -> Vec<u8> {
    let row = (FB_WIDTH * 3).div_ceil(4) * 4;
    let size = row * FB_HEIGHT;
    let mut out = Vec::with_capacity(54 + size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + size) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(FB_WIDTH as i32).to_le_bytes());
    out.extend_from_slice(&(FB_HEIGHT as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    for y in (0..FB_HEIGHT).rev() {
        let start = out.len();
        for x in 0..FB_WIDTH {
            let i = (y * FB_WIDTH + x) * 4;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i]]);
        }
        out.resize(start + row, 0);
    }
    out
}

/// Hash FNV-1a de 64 bits (determinista) para comparar estados en CI.
pub fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325u64, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fb() -> Vec<u8> {
        let mut v = vec![0u8; FB_WIDTH * FB_HEIGHT * 4];
        v[0..4].copy_from_slice(&[1, 2, 3, 255]); // píxel (0,0)
        let last = v.len() - 4;
        v[last..].copy_from_slice(&[10, 20, 30, 255]); // píxel (351, 295)
        v
    }

    #[test]
    fn ppm_header_and_size() {
        let p = to_ppm(&fb());
        assert!(p.starts_with(b"P6\n352 296\n255\n"));
        assert_eq!(p.len(), 15 + FB_WIDTH * FB_HEIGHT * 3);
        assert_eq!(&p[15..18], &[1, 2, 3]);
    }

    #[test]
    fn bmp_layout() {
        let b = to_bmp(&fb());
        assert_eq!(&b[..2], b"BM");
        assert_eq!(
            u32::from_le_bytes(b[2..6].try_into().unwrap()) as usize,
            b.len()
        );
        assert_eq!(i32::from_le_bytes(b[18..22].try_into().unwrap()), 352);
        assert_eq!(i32::from_le_bytes(b[22..26].try_into().unwrap()), 296);
        // Primera fila almacenada = última fila de la imagen: su último píxel en BGR.
        let row = FB_WIDTH * 3;
        assert_eq!(&b[54 + row - 3..54 + row], &[30, 20, 10]);
        // Última fila almacenada = fila superior: píxel (0,0) en BGR.
        let last_row = 54 + (FB_HEIGHT - 1) * row;
        assert_eq!(&b[last_row..last_row + 3], &[3, 2, 1]);
    }

    #[test]
    fn fnv_known_values() {
        assert_eq!(fnv1a(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a(b"a"), 0xaf63dc4c8601ec8c);
    }
}
