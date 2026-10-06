//! Traducción del teclado físico a teclas del Spectrum y al joystick Kempston.
//!
//! Modelo "por estado": en cada frame de la interfaz se calcula el conjunto de teclas que
//! deberían estar pulsadas y se compara con el anterior; solo las diferencias llegan al core.
//! Así la repetición automática del sistema operativo no genera pulsaciones y la pérdida de
//! foco libera todo (conjunto vacío). Mapeo posicional: cada tecla del PC equivale a la
//! tecla del Spectrum de su misma posición, más equivalencias de comodidad:
//!
//! * Shift → Caps Shift; Ctrl o Alt → Symbol Shift.
//! * Retroceso → Caps Shift + 0 (borrar); flechas → teclas de cursor (Caps Shift + 5/6/7/8),
//!   o joystick Kempston si está conectado (disparo: Insert o Fin).
//! * `, . ; - = / '` sin Shift → Symbol Shift + N, M, O, J, L, V, 7.

use crate::input::kempston::JoyButton;
use crate::input::keyboard::SpectrumKey;
use eframe::egui::{Key, Modifiers};
use std::collections::HashSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub spectrum: HashSet<SpectrumKey>,
    pub joy: HashSet<JoyButton>,
}

/// Tecla de letra/dígito/enter/espacio → tecla única del Spectrum.
pub fn base_key(k: Key) -> Option<SpectrumKey> {
    use SpectrumKey as S;
    Some(match k {
        Key::A => S::A,
        Key::B => S::B,
        Key::C => S::C,
        Key::D => S::D,
        Key::E => S::E,
        Key::F => S::F,
        Key::G => S::G,
        Key::H => S::H,
        Key::I => S::I,
        Key::J => S::J,
        Key::K => S::K,
        Key::L => S::L,
        Key::M => S::M,
        Key::N => S::N,
        Key::O => S::O,
        Key::P => S::P,
        Key::Q => S::Q,
        Key::R => S::R,
        Key::S => S::S,
        Key::T => S::T,
        Key::U => S::U,
        Key::V => S::V,
        Key::W => S::W,
        Key::X => S::X,
        Key::Y => S::Y,
        Key::Z => S::Z,
        Key::Num0 => S::N0,
        Key::Num1 => S::N1,
        Key::Num2 => S::N2,
        Key::Num3 => S::N3,
        Key::Num4 => S::N4,
        Key::Num5 => S::N5,
        Key::Num6 => S::N6,
        Key::Num7 => S::N7,
        Key::Num8 => S::N8,
        Key::Num9 => S::N9,
        Key::Enter => S::Enter,
        Key::Space => S::Space,
        _ => return None,
    })
}

/// Teclas de comodidad: (modificador, tecla) del Spectrum que equivalen a una tecla del PC.
fn combo(k: Key, shift_held: bool) -> Option<&'static [SpectrumKey]> {
    use SpectrumKey as S;
    match k {
        Key::Backspace => Some(&[S::CapsShift, S::N0]),
        // Signos de puntuación: solo sin Shift (con Shift el PC produciría otro carácter).
        Key::Comma if !shift_held => Some(&[S::SymbolShift, S::N]),
        Key::Period if !shift_held => Some(&[S::SymbolShift, S::M]),
        Key::Semicolon if !shift_held => Some(&[S::SymbolShift, S::O]),
        Key::Minus if !shift_held => Some(&[S::SymbolShift, S::J]),
        Key::Equals if !shift_held => Some(&[S::SymbolShift, S::L]),
        Key::Slash if !shift_held => Some(&[S::SymbolShift, S::V]),
        Key::Quote if !shift_held => Some(&[S::SymbolShift, S::N7]),
        _ => None,
    }
}

fn cursor_combo(k: Key) -> Option<&'static [SpectrumKey]> {
    use SpectrumKey as S;
    match k {
        Key::ArrowLeft => Some(&[S::CapsShift, S::N5]),
        Key::ArrowDown => Some(&[S::CapsShift, S::N6]),
        Key::ArrowUp => Some(&[S::CapsShift, S::N7]),
        Key::ArrowRight => Some(&[S::CapsShift, S::N8]),
        _ => None,
    }
}

fn joy_button(k: Key) -> Option<JoyButton> {
    match k {
        Key::ArrowLeft => Some(JoyButton::Left),
        Key::ArrowRight => Some(JoyButton::Right),
        Key::ArrowUp => Some(JoyButton::Up),
        Key::ArrowDown => Some(JoyButton::Down),
        Key::Insert | Key::End => Some(JoyButton::Fire),
        _ => None,
    }
}

/// Conjunto de teclas que deben estar pulsadas dados las teclas físicas y los modificadores.
pub fn held_from(keys: &HashSet<Key>, mods: Modifiers, kempston: bool) -> Held {
    let mut h = Held::default();
    if mods.shift {
        h.spectrum.insert(SpectrumKey::CapsShift);
    }
    if mods.ctrl || mods.alt {
        h.spectrum.insert(SpectrumKey::SymbolShift);
    }
    for &k in keys {
        if let Some(sk) = base_key(k) {
            h.spectrum.insert(sk);
        } else if let Some(c) = combo(k, mods.shift) {
            h.spectrum.extend(c.iter().copied());
        } else if kempston {
            if let Some(b) = joy_button(k) {
                h.joy.insert(b);
            }
        } else if let Some(c) = cursor_combo(k) {
            h.spectrum.extend(c.iter().copied());
        }
    }
    h
}

/// Diferencias entre dos estados: (teclas a soltar, teclas a pulsar) y lo mismo para el joystick.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub key_up: Vec<SpectrumKey>,
    pub key_down: Vec<SpectrumKey>,
    pub joy_up: Vec<JoyButton>,
    pub joy_down: Vec<JoyButton>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.key_up.is_empty()
            && self.key_down.is_empty()
            && self.joy_up.is_empty()
            && self.joy_down.is_empty()
    }
}

pub fn diff(prev: &Held, next: &Held) -> Diff {
    let mut d = Diff::default();
    // Orden estable (para tests y para que los eventos sean deterministas).
    let order = |k: &SpectrumKey| {
        SpectrumKey::ALL
            .iter()
            .position(|x| x == k)
            .unwrap_or(usize::MAX)
    };
    d.key_up = prev.spectrum.difference(&next.spectrum).copied().collect();
    d.key_down = next.spectrum.difference(&prev.spectrum).copied().collect();
    d.key_up.sort_by_key(order);
    d.key_down.sort_by_key(order);
    let jorder = |b: &JoyButton| *b as usize;
    d.joy_up = prev.joy.difference(&next.joy).copied().collect();
    d.joy_down = next.joy.difference(&prev.joy).copied().collect();
    d.joy_up.sort_by_key(jorder);
    d.joy_down.sort_by_key(jorder);
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use SpectrumKey as S;

    fn keys(ks: &[Key]) -> HashSet<Key> {
        ks.iter().copied().collect()
    }

    fn sp(h: &Held) -> Vec<SpectrumKey> {
        let mut v: Vec<_> = h.spectrum.iter().copied().collect();
        v.sort_by_key(|k| SpectrumKey::ALL.iter().position(|x| x == k));
        v
    }

    #[test]
    fn every_letter_digit_enter_space_maps_to_a_distinct_spectrum_key() {
        let all = [
            Key::A,
            Key::B,
            Key::C,
            Key::D,
            Key::E,
            Key::F,
            Key::G,
            Key::H,
            Key::I,
            Key::J,
            Key::K,
            Key::L,
            Key::M,
            Key::N,
            Key::O,
            Key::P,
            Key::Q,
            Key::R,
            Key::S,
            Key::T,
            Key::U,
            Key::V,
            Key::W,
            Key::X,
            Key::Y,
            Key::Z,
            Key::Num0,
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
            Key::Enter,
            Key::Space,
        ];
        let mut seen = HashSet::new();
        for k in all {
            let sk = base_key(k).unwrap_or_else(|| panic!("{k:?} sin mapear"));
            assert!(seen.insert(sk), "{sk:?} repetida");
        }
        assert_eq!(seen.len(), 38); // 40 teclas menos las dos de modificador
    }

    #[test]
    fn modifiers_map_to_shifts() {
        let h = held_from(&keys(&[]), Modifiers::SHIFT, false);
        assert_eq!(sp(&h), vec![S::CapsShift]);
        let h = held_from(&keys(&[]), Modifiers::CTRL, false);
        assert_eq!(sp(&h), vec![S::SymbolShift]);
        let h = held_from(&keys(&[]), Modifiers::ALT, false);
        assert_eq!(sp(&h), vec![S::SymbolShift]);
        let h = held_from(&keys(&[Key::Num0]), Modifiers::SHIFT, false);
        assert_eq!(sp(&h), vec![S::CapsShift, S::N0]);
    }

    #[test]
    fn backspace_is_caps_shift_zero() {
        let h = held_from(&keys(&[Key::Backspace]), Modifiers::NONE, false);
        assert_eq!(sp(&h), vec![S::CapsShift, S::N0]);
    }

    #[test]
    fn punctuation_conveniences_only_without_shift() {
        let h = held_from(&keys(&[Key::Comma]), Modifiers::NONE, false);
        assert_eq!(sp(&h), vec![S::SymbolShift, S::N]);
        let h = held_from(&keys(&[Key::Quote]), Modifiers::NONE, false);
        assert_eq!(sp(&h), vec![S::N7, S::SymbolShift]);
        let h = held_from(&keys(&[Key::Comma]), Modifiers::SHIFT, false);
        assert_eq!(sp(&h), vec![S::CapsShift]);
    }

    #[test]
    fn arrows_are_cursor_keys_or_kempston_joystick() {
        let h = held_from(&keys(&[Key::ArrowLeft]), Modifiers::NONE, false);
        assert_eq!(sp(&h), vec![S::CapsShift, S::N5]);
        assert!(h.joy.is_empty());
        let h = held_from(&keys(&[Key::ArrowRight]), Modifiers::NONE, false);
        assert_eq!(sp(&h), vec![S::CapsShift, S::N8]);
        let h = held_from(&keys(&[Key::ArrowLeft, Key::Insert]), Modifiers::NONE, true);
        assert!(h.spectrum.is_empty());
        assert_eq!(
            h.joy,
            [JoyButton::Left, JoyButton::Fire].into_iter().collect()
        );
        let h = held_from(&keys(&[Key::End]), Modifiers::NONE, false);
        assert!(
            h.spectrum.is_empty() && h.joy.is_empty(),
            "Fin solo es disparo con Kempston"
        );
    }

    #[test]
    fn unmapped_keys_do_nothing() {
        let h = held_from(
            &keys(&[Key::F5, Key::Escape, Key::Tab, Key::PageUp, Key::Home]),
            Modifiers::NONE,
            false,
        );
        assert_eq!(h, Held::default());
    }

    #[test]
    fn diff_reports_only_changes_in_a_stable_order() {
        let a = held_from(&keys(&[Key::A, Key::B]), Modifiers::NONE, false);
        let b = held_from(&keys(&[Key::B, Key::C]), Modifiers::SHIFT, false);
        let d = diff(&a, &b);
        assert_eq!(d.key_up, vec![S::A]);
        assert_eq!(d.key_down, vec![S::CapsShift, S::C]);
        assert!(d.joy_up.is_empty() && d.joy_down.is_empty());
        assert!(diff(&a, &a).is_empty());
    }

    #[test]
    fn losing_focus_releases_everything() {
        let pressed = held_from(&keys(&[Key::Q, Key::P]), Modifiers::CTRL, false);
        let released = Held::default();
        let d = diff(&pressed, &released);
        assert_eq!(d.key_up.len(), 3);
        assert!(d.key_down.is_empty());
    }

    #[test]
    fn held_keys_do_not_generate_repeats() {
        let a = held_from(&keys(&[Key::Space]), Modifiers::NONE, false);
        // Mismo estado en frames sucesivos (repetición automática del SO): sin diferencias.
        assert!(diff(&a, &a.clone()).is_empty());
    }
}
