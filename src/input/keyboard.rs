//! Matriz de teclado 8×5 del Spectrum. El hardware no conoce caracteres, solo posiciones.

/// Teclas físicas del Spectrum 48K (40 teclas).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpectrumKey {
    CapsShift,
    Z,
    X,
    C,
    V,
    A,
    S,
    D,
    F,
    G,
    Q,
    W,
    E,
    R,
    T,
    N1,
    N2,
    N3,
    N4,
    N5,
    N0,
    N9,
    N8,
    N7,
    N6,
    P,
    O,
    I,
    U,
    Y,
    Enter,
    L,
    K,
    J,
    H,
    Space,
    SymbolShift,
    M,
    N,
    B,
}

impl SpectrumKey {
    pub const ALL: [SpectrumKey; 40] = {
        use SpectrumKey::*;
        [
            CapsShift,
            Z,
            X,
            C,
            V,
            A,
            S,
            D,
            F,
            G,
            Q,
            W,
            E,
            R,
            T,
            N1,
            N2,
            N3,
            N4,
            N5,
            N0,
            N9,
            N8,
            N7,
            N6,
            P,
            O,
            I,
            U,
            Y,
            Enter,
            L,
            K,
            J,
            H,
            Space,
            SymbolShift,
            M,
            N,
            B,
        ]
    };

    /// (fila, bit de columna). El orden de `ALL` es exactamente fila a fila, bit 0 a bit 4.
    pub fn position(self) -> (usize, u8) {
        let idx = Self::ALL.iter().position(|&k| k == self).unwrap_or(0);
        (idx / 5, (idx % 5) as u8)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keyboard {
    /// 8 filas × 5 columnas activas en bajo: 1 = suelta, 0 = pulsada. Solo bits 0..=4.
    rows: [u8; 8],
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyboard {
    pub fn new() -> Self {
        Self { rows: [0x1F; 8] }
    }

    pub fn key_down(&mut self, key: SpectrumKey) {
        let (row, bit) = key.position();
        self.rows[row] &= !(1 << bit);
    }

    pub fn key_up(&mut self, key: SpectrumKey) {
        let (row, bit) = key.position();
        self.rows[row] |= 1 << bit;
    }

    pub fn release_all(&mut self) {
        self.rows = [0x1F; 8];
    }

    /// Lectura según A15..A8 (`high`): cada bit a 0 selecciona una fila; las filas
    /// seleccionadas se combinan con AND (lógica activa en bajo). Devuelve D0..D4.
    pub fn read(&self, high: u8) -> u8 {
        let mut v = 0x1F;
        for (i, row) in self.rows.iter().enumerate() {
            if high & (1 << i) == 0 {
                v &= row;
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SpectrumKey::*;

    #[test]
    fn matrix_layout_matches_hardware() {
        // Semifilas del AGENTS.md: dirección alta -> teclas D0..D4.
        let rows: [(u8, [SpectrumKey; 5]); 8] = [
            (0xFE, [CapsShift, Z, X, C, V]),
            (0xFD, [A, S, D, F, G]),
            (0xFB, [Q, W, E, R, T]),
            (0xF7, [N1, N2, N3, N4, N5]),
            (0xEF, [N0, N9, N8, N7, N6]),
            (0xDF, [P, O, I, U, Y]),
            (0xBF, [Enter, L, K, J, H]),
            (0x7F, [Space, SymbolShift, M, N, B]),
        ];
        for (high, keys) in rows {
            for (bit, key) in keys.iter().enumerate() {
                let mut kb = Keyboard::new();
                kb.key_down(*key);
                assert_eq!(kb.read(high), 0x1F & !(1 << bit), "{key:?}");
                // No aparece en ninguna otra semifila.
                for other in (0..8).map(|i| !(1u8 << i)).filter(|&h| h != high) {
                    assert_eq!(kb.read(other), 0x1F, "{key:?} visible en {other:#04X}");
                }
            }
        }
    }

    #[test]
    fn all_keys_have_unique_positions() {
        let mut seen = std::collections::HashSet::new();
        for k in SpectrumKey::ALL {
            assert!(seen.insert(k.position()));
        }
        assert_eq!(seen.len(), 40);
    }

    #[test]
    fn nothing_pressed_reads_all_ones() {
        let kb = Keyboard::new();
        assert_eq!(kb.read(0x00), 0x1F);
    }

    #[test]
    fn multiple_selected_rows_are_anded() {
        let mut kb = Keyboard::new();
        kb.key_down(Z); // fila 0, bit 1
        kb.key_down(S); // fila 1, bit 1
        kb.key_down(D); // fila 1, bit 2
        assert_eq!(kb.read(0xFC), 0x1F & !0b00110); // filas 0 y 1
        assert_eq!(kb.read(0xFD), 0x1F & !0b00110); // solo fila 1: S y D
        assert_eq!(kb.read(0xFE), 0x1F & !0b00010); // solo fila 0: Z
        assert_eq!(kb.read(0x00), 0x1F & !0b00110); // todas
    }

    #[test]
    fn shift_plus_key_is_two_independent_bits() {
        let mut kb = Keyboard::new();
        kb.key_down(CapsShift);
        kb.key_down(N0); // CAPS SHIFT + 0 = backspace: lo interpreta la ROM
        assert_eq!(kb.read(0xFE), 0x1E);
        assert_eq!(kb.read(0xEF), 0x1E);
    }

    #[test]
    fn key_up_releases() {
        let mut kb = Keyboard::new();
        kb.key_down(Space);
        kb.key_up(Space);
        assert_eq!(kb, Keyboard::new());
    }
}
