//! Contención de memoria/E/S de la ULA 48K (fuente: Sinclair Wiki, "Contended memory"/"Contended I/O").
//!
//! La ULA pausa a la CPU en el primer T-state (T1) de un acceso a memoria 0x4000..=0x7FFF
//! (y en ciertos T-states de E/S) mientras dibuja las 192 líneas de imagen: durante los
//! 128 T de cada línea el retraso sigue el patrón 6,5,4,3,2,1,0,0 sincronizado con el
//! inicio del área de display.

/// Retraso según la posición dentro de cada grupo de 8 T-states.
pub const PATTERN: [u32; 8] = [6, 5, 4, 3, 2, 1, 0, 0];

use super::timing::{DISPLAY_SCANLINES, TSTATES_PER_SCANLINE};

/// T-states por línea durante los cuales la ULA lee display RAM.
const ACTIVE_TSTATES: u32 = 128;

/// Perfil de temporización de la ULA. Los T-states están referidos a "el primer T-state que
/// empieza con /INT bajo es el 0". Las ULA reales pueden estar en "early" o "late timing"
/// (+1 T, depende de la temperatura): es un parámetro explícito, no un valor oculto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HardwareProfile {
    /// T-state del frame en el que se contiende el primer acceso (retraso 6).
    pub contention_start: u32,
}

impl HardwareProfile {
    /// ULA fría (recién encendida): primer retraso en 14335.
    pub const EARLY: Self = Self {
        contention_start: 14335,
    };
    /// ULA caliente: todo 1 T-state más tarde (14336).
    pub const LATE: Self = Self {
        contention_start: 14336,
    };
}

impl Default for HardwareProfile {
    fn default() -> Self {
        Self::EARLY
    }
}

/// ¿Está esa dirección de CPU en RAM compartida con la ULA (0x4000..=0x7FFF)?
pub fn is_contended_address(addr: u16) -> bool {
    (0x4000..=0x7FFF).contains(&addr)
}

/// ¿Aparece esta dirección en el bus como acceso a RAM contendida? (Alto byte 0x40..=0x7F.)
pub fn is_contended_high_byte(high: u8) -> bool {
    (0x40..=0x7F).contains(&high)
}

/// Retraso en T-states si la CPU empieza un acceso contendido en `frame_tstate`.
pub fn delay(profile: HardwareProfile, frame_tstate: u32) -> u32 {
    let Some(off) = frame_tstate.checked_sub(profile.contention_start) else {
        return 0;
    };
    let line = off / TSTATES_PER_SCANLINE;
    let col = off % TSTATES_PER_SCANLINE;
    if line >= DISPLAY_SCANLINES || col >= ACTIVE_TSTATES {
        return 0;
    }
    PATTERN[(col % 8) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: HardwareProfile = HardwareProfile::EARLY;

    #[test]
    fn first_window_matches_reference_table() {
        // Tabla del Sinclair Wiki.
        let expected = [
            (14334, 0),
            (14335, 6),
            (14336, 5),
            (14337, 4),
            (14338, 3),
            (14339, 2),
            (14340, 1),
            (14341, 0),
            (14342, 0),
            (14343, 6),
            (14344, 5),
            (14345, 4),
            (14346, 3),
            (14347, 2),
            (14348, 1),
            (14349, 0),
            (14350, 0),
        ];
        for (t, d) in expected {
            assert_eq!(delay(P, t), d, "t={t}");
        }
    }

    #[test]
    fn line_window_ends_after_128_tstates_and_resumes_next_line() {
        // Último grupo de la línea: 14335+120..127 y luego 96 T sin retraso.
        assert_eq!(delay(P, 14335 + 120), 6);
        assert_eq!(delay(P, 14335 + 126), 0);
        assert_eq!(delay(P, 14335 + 127), 0);
        for t in 14335 + 128..14335 + 224 {
            assert_eq!(delay(P, t), 0, "t={t}");
        }
        // La wiki: "empieza otra vez en 14559".
        assert_eq!(delay(P, 14559), 6);
        assert_eq!(delay(P, 14559 + 3), 3);
    }

    #[test]
    fn contention_covers_exactly_192_lines() {
        let last_line_start = 14335 + 191 * 224;
        assert_eq!(delay(P, last_line_start), 6);
        assert_eq!(delay(P, last_line_start + 224), 0);
        assert_eq!(delay(P, 0), 0);
        assert_eq!(delay(P, 69_887), 0);
    }

    #[test]
    fn late_profile_shifts_by_one() {
        assert_eq!(delay(HardwareProfile::LATE, 14335), 0);
        assert_eq!(delay(HardwareProfile::LATE, 14336), 6);
    }

    #[test]
    fn contended_ranges() {
        assert!(!is_contended_address(0x3FFF));
        assert!(is_contended_address(0x4000));
        assert!(is_contended_address(0x7FFF));
        assert!(!is_contended_address(0x8000));
        assert!(is_contended_high_byte(0x40) && is_contended_high_byte(0x7F));
        assert!(!is_contended_high_byte(0x3F) && !is_contended_high_byte(0x80));
    }
}
