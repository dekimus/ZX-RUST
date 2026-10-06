//! Preferencias persistentes de la aplicación (formato `clave=valor`, tolerante a errores).
//! Lógica pura salvo `load`/`save`, que solo tocan el fichero de configuración del usuario.

use super::viewport::{BorderMode, ScaleMode};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Volumen maestro 0.0..=1.0.
    pub volume: f32,
    pub muted: bool,
    pub scale_mode: ScaleMode,
    pub border_mode: BorderMode,
    /// Filtro lineal (suave) en lugar de vecino más cercano (píxeles nítidos).
    pub linear_filter: bool,
    pub kempston: bool,
    pub tape_autoplay: bool,
    /// Mezcla el sonido de la señal de cinta ("sonido de carga").
    pub tape_sound: bool,
    pub show_status_bar: bool,
    pub show_debug: bool,
    pub rom_path: Option<PathBuf>,
    pub allow_nonstandard_rom: bool,
    /// Último directorio usado en los selectores de archivo.
    pub last_dir: Option<PathBuf>,
    /// Escala entera inicial de la ventana (1..=6).
    pub window_scale: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            volume: 0.8,
            muted: false,
            scale_mode: ScaleMode::default(),
            border_mode: BorderMode::default(),
            linear_filter: false,
            kempston: false,
            tape_autoplay: true,
            tape_sound: true,
            show_status_bar: true,
            show_debug: false,
            rom_path: None,
            allow_nonstandard_rom: false,
            last_dir: None,
            window_scale: 2,
        }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl Settings {
    pub fn to_text(&self) -> String {
        let b = |v: bool| if v { "1" } else { "0" };
        let mut s = String::from("# Configuración de zx48 (se genera automáticamente)\n");
        s += &format!("volume={:.2}\n", self.volume);
        s += &format!("muted={}\n", b(self.muted));
        s += &format!("scale_mode={}\n", self.scale_mode.key());
        s += &format!("border_mode={}\n", self.border_mode.key());
        s += &format!("linear_filter={}\n", b(self.linear_filter));
        s += &format!("kempston={}\n", b(self.kempston));
        s += &format!("tape_autoplay={}\n", b(self.tape_autoplay));
        s += &format!("tape_sound={}\n", b(self.tape_sound));
        s += &format!("show_status_bar={}\n", b(self.show_status_bar));
        s += &format!("show_debug={}\n", b(self.show_debug));
        s += &format!("allow_nonstandard_rom={}\n", b(self.allow_nonstandard_rom));
        s += &format!("window_scale={}\n", self.window_scale);
        if let Some(p) = &self.rom_path {
            s += &format!("rom_path={}\n", p.display());
        }
        if let Some(p) = &self.last_dir {
            s += &format!("last_dir={}\n", p.display());
        }
        s
    }

    /// Interpreta un texto de configuración. Las líneas desconocidas o inválidas se ignoran y
    /// conservan el valor por defecto; los valores numéricos se limitan a su rango.
    pub fn from_text(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "volume" => {
                    if let Ok(x) = v.parse::<f32>() {
                        if x.is_finite() {
                            s.volume = x.clamp(0.0, 1.0);
                        }
                    }
                }
                "muted" => s.muted = parse_bool(v).unwrap_or(s.muted),
                "scale_mode" => s.scale_mode = ScaleMode::from_key(v).unwrap_or(s.scale_mode),
                "border_mode" => s.border_mode = BorderMode::from_key(v).unwrap_or(s.border_mode),
                "linear_filter" => s.linear_filter = parse_bool(v).unwrap_or(s.linear_filter),
                "kempston" => s.kempston = parse_bool(v).unwrap_or(s.kempston),
                "tape_autoplay" => s.tape_autoplay = parse_bool(v).unwrap_or(s.tape_autoplay),
                "tape_sound" => s.tape_sound = parse_bool(v).unwrap_or(s.tape_sound),
                "show_status_bar" => s.show_status_bar = parse_bool(v).unwrap_or(s.show_status_bar),
                "show_debug" => s.show_debug = parse_bool(v).unwrap_or(s.show_debug),
                "allow_nonstandard_rom" => {
                    s.allow_nonstandard_rom = parse_bool(v).unwrap_or(s.allow_nonstandard_rom)
                }
                "window_scale" => {
                    if let Ok(x) = v.parse::<u32>() {
                        s.window_scale = x.clamp(1, 6);
                    }
                }
                "rom_path" if !v.is_empty() => s.rom_path = Some(PathBuf::from(v)),
                "last_dir" if !v.is_empty() => s.last_dir = Some(PathBuf::from(v)),
                _ => {}
            }
        }
        s
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|t| Self::from_text(&t))
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_text())
    }

    pub fn load() -> Self {
        config_path()
            .map(|p| Self::load_from(&p))
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        match config_path() {
            Some(p) => self.save_to(&p),
            None => Ok(()),
        }
    }
}

/// Ruta del fichero de configuración según la plataforma (sin dependencias adicionales).
pub fn config_path() -> Option<PathBuf> {
    config_dir_from(|k| std::env::var_os(k)).map(|d| d.join("zx48").join("settings.conf"))
}

fn config_dir_from(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        env("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| PathBuf::from(h).join("Library").join("Application Support"))
    } else {
        env("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".config")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let s = Settings::default();
        assert_eq!(s.volume, 0.8);
        assert!(!s.muted && s.tape_autoplay && s.tape_sound && s.show_status_bar && !s.show_debug);
        assert_eq!(s.border_mode, BorderMode::Medium);
        assert_eq!(s.scale_mode, ScaleMode::Integer);
        assert!(!s.linear_filter, "por defecto vecino más cercano");
    }

    #[test]
    fn text_round_trip() {
        let s = Settings {
            volume: 0.35,
            muted: true,
            scale_mode: ScaleMode::Fit,
            border_mode: BorderMode::Full,
            linear_filter: true,
            kempston: true,
            tape_autoplay: false,
            tape_sound: false,
            show_status_bar: false,
            show_debug: true,
            rom_path: Some("/tmp/48 rom.rom".into()),
            allow_nonstandard_rom: true,
            last_dir: Some("/home/x/juegos".into()),
            window_scale: 3,
        };
        assert_eq!(Settings::from_text(&s.to_text()), s);
    }

    #[test]
    fn invalid_input_falls_back_to_defaults_and_clamps() {
        let s = Settings::from_text(
            "volume=7\nmuted=maybe\nborder_mode=huge\nwindow_scale=99\nnonsense\n=\n#comentario\nunknown=1\nkempston=1\nrom_path=\n",
        );
        let d = Settings::default();
        assert_eq!(s.volume, 1.0);
        assert_eq!(s.muted, d.muted);
        assert_eq!(s.border_mode, d.border_mode);
        assert_eq!(s.window_scale, 6);
        assert!(s.kempston);
        assert_eq!(s.rom_path, None);
        assert_eq!(Settings::from_text("volume=-3").volume, 0.0);
        assert_eq!(Settings::from_text("volume=nan").volume, d.volume);
        assert_eq!(Settings::from_text("window_scale=0").window_scale, 1);
        assert_eq!(Settings::from_text("\u{0}\u{1}garbage"), d);
    }

    #[test]
    fn save_and_load_through_a_file() {
        let dir = std::env::temp_dir()
            .join("zx48-settings-test")
            .join("nested");
        let path = dir.join("settings.conf");
        let _ = std::fs::remove_file(&path);
        let mut s = Settings::default();
        s.volume = 0.5;
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        // Un fichero que no existe da los valores por defecto, sin error.
        assert_eq!(
            Settings::load_from(&dir.join("missing.conf")),
            Settings::default()
        );
    }

    #[test]
    fn config_dir_resolution() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| std::ffi::OsString::from(v))
            }
        };
        if cfg!(all(unix, not(target_os = "macos"))) {
            assert_eq!(
                config_dir_from(env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/u")])),
                Some(PathBuf::from("/xdg"))
            );
            assert_eq!(
                config_dir_from(env(&[("HOME", "/home/u")])),
                Some(PathBuf::from("/home/u/.config"))
            );
            assert_eq!(
                config_dir_from(env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")])),
                Some(PathBuf::from("/h/.config"))
            );
            assert_eq!(config_dir_from(env(&[])), None);
        }
    }
}
