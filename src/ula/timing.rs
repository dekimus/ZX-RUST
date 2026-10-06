pub const CPU_HZ: u32 = 3_500_000;
pub const TSTATES_PER_SCANLINE: u32 = 224;
pub const SCANLINES_PER_FRAME: u32 = 312;
pub const TSTATES_PER_FRAME: u32 = TSTATES_PER_SCANLINE * SCANLINES_PER_FRAME;
/// Scanlines antes de la primera línea de imagen.
pub const SCANLINES_BEFORE_DISPLAY: u32 = 64;
pub const DISPLAY_SCANLINES: u32 = 192;
/// T-states desde la interrupción hasta el primer byte visible.
pub const FIRST_DISPLAY_TSTATE: u32 = SCANLINES_BEFORE_DISPLAY * TSTATES_PER_SCANLINE;
/// Duración del pulso /INT en el 48K.
pub const INT_LENGTH: u32 = 32;

/// T-state dentro del frame para un contador maestro absoluto.
pub fn tstate_in_frame(master: u64) -> u32 {
    (master % TSTATES_PER_FRAME as u64) as u32
}

/// /INT activa durante los primeros 32 T-states de cada frame.
pub fn int_active(master: u64) -> bool {
    tstate_in_frame(master) < INT_LENGTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_constants() {
        assert_eq!(TSTATES_PER_FRAME, 69_888);
        assert_eq!(FIRST_DISPLAY_TSTATE, 14_336);
    }

    #[test]
    fn int_window() {
        assert!(int_active(0));
        assert!(int_active(31));
        assert!(!int_active(32));
        assert!(int_active(TSTATES_PER_FRAME as u64));
    }
}
