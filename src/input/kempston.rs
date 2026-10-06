//! Interfaz de joystick Kempston (periférico opcional; el 48K original no la incluye).
//!
//! Lectura por puertos con A5 = 0 (el estándar de facto decodifica `port & 0xE0 == 0`, p. ej. 0x1F).
//! Bits activos en ALTO: D0 derecha, D1 izquierda, D2 abajo, D3 arriba, D4 disparo; D5..D7 = 0.
//!
//! Con la interfaz desconectada el rango devuelve el floating bus con D0..D4 forzados a 0
//! ([`Kempston::absent_value`]): no hay interfaz, luego ningún botón puede estar pulsado, de modo
//! que el software que arrancó con la interfaz presente (snapshot con el flag "interfaz detectada"
//! ya en RAM) no ve entradas fantasma; D5..D7 siguen el floating bus, así que las rutinas de
//! detección que exigen esos bits a 0 siguen viendo "sin interfaz", como en un 48K real.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JoyButton {
    Right,
    Left,
    Down,
    Up,
    Fire,
}

impl JoyButton {
    fn bit(self) -> u8 {
        match self {
            Self::Right => 0x01,
            Self::Left => 0x02,
            Self::Down => 0x04,
            Self::Up => 0x08,
            Self::Fire => 0x10,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Kempston {
    enabled: bool,
    state: u8,
}

impl Kempston {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    pub fn press(&mut self, b: JoyButton) {
        self.state |= b.bit();
    }

    pub fn release(&mut self, b: JoyButton) {
        self.state &= !b.bit();
    }

    pub fn release_all(&mut self) {
        self.state = 0;
    }

    /// ¿Está el puerto dentro del rango que decodifica la interfaz (conectada o no)?
    pub fn in_range(port: u16) -> bool {
        port & 0x00E0 == 0
    }

    /// ¿Responde la interfaz a este puerto?
    pub fn decodes(&self, port: u16) -> bool {
        self.enabled && Self::in_range(port)
    }

    pub fn read(&self) -> u8 {
        self.state
    }

    /// Lectura del rango Kempston con la interfaz desconectada.
    ///
    /// `floating` es el valor del floating bus para ese T-state. D0..D4 (botones) se leen 0:
    /// sin interfaz no hay botones que pulsar, y un `0xFF` completo lo interpretaría el software
    /// con el flag de interfaz detectada en RAM como "todo pulsado" (entrada fantasma).
    /// D5..D7 conservan el floating bus, que es donde la detección de interfaz distingue
    /// "ausente" (bits a 1 en reposo) de "conectada" (siempre 0).
    pub fn absent_value(floating: u8) -> u8 {
        floating & 0xE0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_and_decoding() {
        let mut k = Kempston::new();
        assert!(!k.decodes(0x001F));
        k.set_enabled(true);
        assert!(k.decodes(0x001F) && k.decodes(0x011F) && k.decodes(0xFF1F));
        assert!(!k.decodes(0x00FE) && !k.decodes(0x00FF) && !k.decodes(0x0020));
        assert_eq!(k.read(), 0);
        k.press(JoyButton::Right);
        k.press(JoyButton::Fire);
        assert_eq!(k.read(), 0x11);
        k.release(JoyButton::Right);
        assert_eq!(k.read(), 0x10);
        k.press(JoyButton::Up);
        k.press(JoyButton::Left);
        k.press(JoyButton::Down);
        assert_eq!(k.read(), 0x1E);
        k.release_all();
        assert_eq!(k.read(), 0);
    }

    #[test]
    fn absent_range_hides_buttons_but_keeps_floating_bus_bits() {
        assert!(Kempston::in_range(0x001F) && Kempston::in_range(0x011F));
        assert!(!Kempston::in_range(0x00FE) && !Kempston::in_range(0x00FF));
        assert!(!Kempston::in_range(0x0020) && !Kempston::in_range(0xFFE0));
        // Reposo (border): floating 0xFF -> sin botones, ausencia visible en D5..D7.
        assert_eq!(Kempston::absent_value(0xFF), 0xE0);
        // Fetch de vídeo: se conservan D5..D7 del byte leído y se limpian los bits de botón.
        assert_eq!(Kempston::absent_value(0xB0), 0xA0);
        assert_eq!(Kempston::absent_value(0x1F), 0x00);
        // Una interfaz conectada no pasa por aquí: su lectura es el estado real.
        let mut k = Kempston::new();
        k.set_enabled(true);
        k.press(JoyButton::Right);
        assert_eq!(k.read(), 0x01);
    }
}
