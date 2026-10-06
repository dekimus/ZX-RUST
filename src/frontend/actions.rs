//! Acciones de la aplicación y atajos de teclado.
//!
//! Estrategia de atajos: solo teclas de función (F1..F12) sin modificadores y PageUp/PageDown;
//! así no chocan con el teclado del Spectrum (letras, números, Shift, Ctrl/Alt = Symbol Shift).

use eframe::egui::{self, Key};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    LoadTape,
    LoadSnapshot,
    SaveSnapshot,
    SaveScreenshot,
    SelectRom,
    Reset,
    TogglePause,
    StepFrame,
    TapePlayStop,
    TapeRewind,
    TapeEject,
    ToggleMute,
    VolumeUp,
    VolumeDown,
    ToggleFullscreen,
    ToggleDebug,
    ShowHelp,
    ShowAbout,
    Quit,
}

/// Tabla única de atajos (se usa para el despacho y para las etiquetas de los menús).
pub const SHORTCUTS: &[(Key, Action, &str)] = &[
    (Key::F1, Action::ShowHelp, "F1"),
    (Key::F2, Action::LoadTape, "F2"),
    (Key::F3, Action::LoadSnapshot, "F3"),
    (Key::F4, Action::Reset, "F4"),
    (Key::F5, Action::TogglePause, "F5"),
    (Key::F6, Action::TapePlayStop, "F6"),
    (Key::F7, Action::TapeRewind, "F7"),
    (Key::F8, Action::ToggleMute, "F8"),
    (Key::F9, Action::SaveSnapshot, "F9"),
    (Key::F10, Action::ToggleDebug, "F10"),
    (Key::F11, Action::ToggleFullscreen, "F11"),
    (Key::F12, Action::SaveScreenshot, "F12"),
    (Key::PageUp, Action::VolumeUp, "Re Pág"),
    (Key::PageDown, Action::VolumeDown, "Av Pág"),
];

/// Acción asociada a una pulsación (solo sin Ctrl/Alt/Cmd; Shift se tolera porque Shift solo
/// es Caps Shift para el Spectrum).
pub fn shortcut(key: Key, mods: egui::Modifiers) -> Option<Action> {
    if mods.ctrl || mods.alt || mods.command || mods.mac_cmd {
        return None;
    }
    SHORTCUTS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, a, _)| *a)
}

/// Ventana de anti-rebote de los atajos.
///
/// Algunos servidores X envían la auto-repetición como pares press/release sin marcarla como
/// repetición: con `hold` largo, una sola pulsación de F5 llegaría dos veces (pausa → reanudar
/// en el mismo frame). Se ignora el mismo atajo repetido dentro de esta ventana; las acciones
/// de volumen se excluyen porque su repetición al mantener la tecla es deseable.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// `true` si `action` debe descartarse por haberse despachado hace menos de [`DEBOUNCE`].
pub fn debounced(last: Option<(Action, Instant)>, action: Action, now: Instant) -> bool {
    if matches!(action, Action::VolumeUp | Action::VolumeDown) {
        return false;
    }
    let Some((prev, at)) = last else { return false };
    if prev != action {
        return false;
    }
    // Un `at` en el futuro (reloj no monotónico) también se trata como rebote.
    now.checked_duration_since(at).is_none_or(|d| d < DEBOUNCE)
}

pub fn shortcut_label(action: Action) -> Option<&'static str> {
    SHORTCUTS
        .iter()
        .find(|(_, a, _)| *a == action)
        .map(|(_, _, l)| *l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Modifiers;

    #[test]
    fn function_keys_map_to_actions() {
        assert_eq!(
            shortcut(Key::F5, Modifiers::NONE),
            Some(Action::TogglePause)
        );
        assert_eq!(shortcut(Key::F4, Modifiers::NONE), Some(Action::Reset));
        assert_eq!(
            shortcut(Key::F11, Modifiers::NONE),
            Some(Action::ToggleFullscreen)
        );
        assert_eq!(
            shortcut(Key::PageUp, Modifiers::NONE),
            Some(Action::VolumeUp)
        );
        // Shift sola no impide el atajo.
        assert_eq!(shortcut(Key::F2, Modifiers::SHIFT), Some(Action::LoadTape));
    }

    #[test]
    fn spectrum_keys_never_trigger_actions() {
        for k in [
            Key::A,
            Key::Z,
            Key::Num0,
            Key::Enter,
            Key::Space,
            Key::Backspace,
            Key::Escape,
            Key::Tab,
        ] {
            assert_eq!(shortcut(k, Modifiers::NONE), None, "{k:?}");
        }
    }

    #[test]
    fn ctrl_alt_combinations_are_left_to_the_spectrum() {
        assert_eq!(shortcut(Key::F5, Modifiers::CTRL), None);
        assert_eq!(shortcut(Key::F5, Modifiers::ALT), None);
    }

    #[test]
    fn debounce_filters_identical_repeats_only() {
        let now = Instant::now();
        let before = now - Duration::from_millis(100);
        assert!(
            !debounced(None, Action::TogglePause, now),
            "primer atajo: nunca se descarta"
        );
        assert!(
            debounced(
                Some((Action::TogglePause, before)),
                Action::TogglePause,
                now
            ),
            "mismo atajo a los 100 ms: rebote"
        );
        assert!(
            !debounced(Some((Action::TogglePause, before)), Action::Reset, now),
            "otra acción distinta: no se filtra"
        );
        let old = now - Duration::from_millis(500);
        assert!(
            !debounced(Some((Action::TogglePause, old)), Action::TogglePause, now),
            "a los 500 ms ya se puede volver a pulsar"
        );
        // El volumen se puede mantener pulsado.
        assert!(!debounced(
            Some((Action::VolumeUp, before)),
            Action::VolumeUp,
            now
        ));
        // Reloj no monotónico: se trata como rebote en lugar de panicar.
        let future = now + Duration::from_secs(10);
        assert!(debounced(Some((Action::Reset, future)), Action::Reset, now));
    }

    #[test]
    fn shortcuts_are_unique_and_labelled() {
        let mut keys = std::collections::HashSet::new();
        let mut actions = std::collections::HashSet::new();
        for (k, a, label) in SHORTCUTS {
            assert!(keys.insert(*k), "tecla repetida {k:?}");
            assert!(actions.insert(*a as usize), "acción repetida {a:?}");
            assert!(!label.is_empty());
        }
        assert_eq!(shortcut_label(Action::Reset), Some("F4"));
        assert_eq!(shortcut_label(Action::Quit), None);
    }
}
