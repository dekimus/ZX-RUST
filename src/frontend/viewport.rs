//! Geometría del área de imagen: región visible del framebuffer según el border y colocación
//! con escalado entero o ajustado a la ventana. Lógica pura (sin ventana) y probada.
//!
//! El framebuffer del core es de 352×296 (256×192 de pantalla + border). La pantalla propiamente
//! dicha mantiene siempre píxeles cuadrados y relación 4:3; el modo de border por defecto
//! (`Medium`, 320×240) es exactamente 4:3.

use crate::ula::{BORDER_BOTTOM, BORDER_LEFT, BORDER_RIGHT, BORDER_TOP, FB_HEIGHT, FB_WIDTH};

/// Rectángulo en píxeles del framebuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Region {
    /// Coordenadas UV normalizadas (u0, v0, u1, v1) sobre la textura completa del framebuffer.
    pub fn uv(&self) -> [f32; 4] {
        let (fw, fh) = (FB_WIDTH as f32, FB_HEIGHT as f32);
        [
            self.x as f32 / fw,
            self.y as f32 / fh,
            (self.x + self.w) as f32 / fw,
            (self.y + self.h) as f32 / fh,
        ]
    }
}

/// Cuánto border se muestra alrededor de la pantalla de 256×192.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BorderMode {
    /// Todo el border que genera la ULA (352×296).
    Full,
    /// 32 px a los lados y 24 arriba/abajo (320×240, exactamente 4:3).
    #[default]
    Medium,
    /// Solo la pantalla (256×192).
    None,
}

impl BorderMode {
    pub const ALL: [BorderMode; 3] = [Self::Full, Self::Medium, Self::None];

    pub fn region(self) -> Region {
        let (sx, sy) = (BORDER_LEFT as u32, BORDER_TOP as u32);
        match self {
            Self::Full => Region {
                x: 0,
                y: 0,
                w: FB_WIDTH as u32,
                h: FB_HEIGHT as u32,
            },
            Self::Medium => Region {
                x: sx - 32,
                y: sy - 24,
                w: 256 + 64,
                h: 192 + 48,
            },
            Self::None => Region {
                x: sx,
                y: sy,
                w: 256,
                h: 192,
            },
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Border completo",
            Self::Medium => "Border reducido (4:3)",
            Self::None => "Sin border",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Medium => "medium",
            Self::None => "none",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScaleMode {
    /// Múltiplos enteros del tamaño original (píxeles perfectos); barras si no encaja exacto.
    #[default]
    Integer,
    /// Escala fraccionaria máxima que cabe, conservando la proporción.
    Fit,
}

impl ScaleMode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Integer => "integer",
            Self::Fit => "fit",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        [Self::Integer, Self::Fit]
            .into_iter()
            .find(|m| m.key() == s)
    }
}

/// Colocación de la imagen dentro del área disponible (todo en píxeles físicos).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub scale: f32,
}

/// Calcula la colocación centrada de una imagen `content_w`×`content_h` en `avail_w`×`avail_h`.
/// Nunca deforma: el factor de escala es el mismo en X e Y.
pub fn place(
    avail_w: f32,
    avail_h: f32,
    content_w: u32,
    content_h: u32,
    mode: ScaleMode,
) -> Placement {
    let (cw, ch) = (content_w as f32, content_h as f32);
    let fit = (avail_w / cw).min(avail_h / ch).max(0.0);
    let scale = match mode {
        // Si ni siquiera cabe a 1×, se reduce de forma fraccionaria (no hay alternativa).
        ScaleMode::Integer if fit >= 1.0 => fit.floor(),
        _ => fit,
    };
    let (w, h) = (cw * scale, ch * scale);
    let (x, y) = ((avail_w - w) / 2.0, (avail_h - h) / 2.0);
    // En escalado entero la imagen se alinea a píxeles físicos para que sea nítida.
    let (x, y) = if mode == ScaleMode::Integer && fit >= 1.0 {
        (x.floor(), y.floor())
    } else {
        (x, y)
    };
    Placement { x, y, w, h, scale }
}

/// Tamaño de ventana (en puntos lógicos) para mostrar el contenido a `scale`× más barras de UI.
pub fn window_size_for(region: Region, scale: u32, ui_height: f32) -> (f32, f32) {
    (
        (region.w * scale) as f32,
        (region.h * scale) as f32 + ui_height,
    )
}

/// Texto "N×" del factor de escala para la barra de estado ("3×", "2.4×").
pub fn scale_label(scale: f32) -> String {
    if (scale - scale.round()).abs() < 1e-3 {
        format!("{}×", scale.round() as u32)
    } else {
        format!("{scale:.2}×")
    }
}

#[allow(dead_code)]
const _: () = {
    // El border completo debe coincidir con las constantes de la ULA.
    assert!(BORDER_LEFT + 256 + BORDER_RIGHT == FB_WIDTH);
    assert!(BORDER_TOP + 192 + BORDER_BOTTOM == FB_HEIGHT);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_are_inside_the_framebuffer_and_contain_the_screen() {
        for m in BorderMode::ALL {
            let r = m.region();
            assert!(
                r.x + r.w <= FB_WIDTH as u32 && r.y + r.h <= FB_HEIGHT as u32,
                "{m:?}"
            );
            // La pantalla de 256×192 (en 48,48) siempre queda dentro.
            assert!(
                r.x <= 48 && r.y <= 48 && r.x + r.w >= 48 + 256 && r.y + r.h >= 48 + 192,
                "{m:?}"
            );
        }
        assert_eq!(
            BorderMode::None.region(),
            Region {
                x: 48,
                y: 48,
                w: 256,
                h: 192
            }
        );
        assert_eq!(
            BorderMode::Full.region(),
            Region {
                x: 0,
                y: 0,
                w: 352,
                h: 296
            }
        );
    }

    #[test]
    fn default_and_none_modes_are_exactly_4_3() {
        for m in [BorderMode::Medium, BorderMode::None] {
            let r = m.region();
            assert_eq!(r.w * 3, r.h * 4, "{m:?}");
        }
        assert_eq!(BorderMode::default(), BorderMode::Medium);
    }

    #[test]
    fn uv_covers_the_selected_region() {
        let uv = BorderMode::None.region().uv();
        assert!((uv[0] - 48.0 / 352.0).abs() < 1e-6 && (uv[1] - 48.0 / 296.0).abs() < 1e-6);
        assert!((uv[2] - 304.0 / 352.0).abs() < 1e-6 && (uv[3] - 240.0 / 296.0).abs() < 1e-6);
        assert_eq!(BorderMode::Full.region().uv(), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn integer_scaling_picks_the_largest_integer_that_fits() {
        let p = place(1000.0, 800.0, 320, 240, ScaleMode::Integer);
        assert_eq!(p.scale, 3.0); // 960×720 cabe; 4× (1280) no
        assert_eq!((p.w, p.h), (960.0, 720.0));
        assert_eq!((p.x, p.y), (20.0, 40.0));
        // Exacto.
        let p = place(640.0, 480.0, 320, 240, ScaleMode::Integer);
        assert_eq!((p.scale, p.x, p.y, p.w, p.h), (2.0, 0.0, 0.0, 640.0, 480.0));
    }

    #[test]
    fn integer_scaling_limited_by_the_smaller_dimension() {
        let p = place(2000.0, 500.0, 256, 192, ScaleMode::Integer);
        assert_eq!(p.scale, 2.0);
        assert_eq!(p.h, 384.0);
        assert_eq!(p.y, 58.0);
        assert_eq!(p.x, 744.0);
    }

    #[test]
    fn integer_positions_are_pixel_aligned() {
        let p = place(1001.0, 801.0, 320, 240, ScaleMode::Integer);
        assert_eq!(p.x, p.x.floor());
        assert_eq!(p.y, p.y.floor());
    }

    #[test]
    fn fit_mode_keeps_aspect_and_centers() {
        let p = place(1000.0, 800.0, 320, 240, ScaleMode::Fit);
        assert!((p.scale - 3.125).abs() < 1e-6);
        assert!((p.w / p.h - 4.0 / 3.0).abs() < 1e-6);
        assert!((p.x * 2.0 + p.w - 1000.0).abs() < 1e-3);
        assert!((p.y * 2.0 + p.h - 800.0).abs() < 1e-3);
        // Nunca deforma, sea cual sea el área.
        for (w, h) in [
            (300.0, 900.0),
            (1920.0, 200.0),
            (50.0, 50.0),
            (777.0, 555.0),
        ] {
            let p = place(w, h, 352, 296, ScaleMode::Fit);
            assert!((p.w / p.h - 352.0 / 296.0).abs() < 1e-4, "{w}x{h}");
            assert!(p.w <= w + 1e-3 && p.h <= h + 1e-3);
        }
    }

    #[test]
    fn smaller_than_original_falls_back_to_fractional_scale_even_in_integer_mode() {
        let p = place(200.0, 150.0, 320, 240, ScaleMode::Integer);
        assert!(p.scale < 1.0 && p.scale > 0.0);
        assert!(p.w <= 200.0 + 1e-3 && p.h <= 150.0 + 1e-3);
    }

    #[test]
    fn degenerate_areas_do_not_panic() {
        let p = place(0.0, 0.0, 320, 240, ScaleMode::Integer);
        assert_eq!((p.w, p.h), (0.0, 0.0));
        let p = place(-5.0, 10.0, 320, 240, ScaleMode::Fit);
        assert_eq!((p.w, p.h), (0.0, 0.0));
    }

    #[test]
    fn keys_round_trip_and_window_size() {
        for m in BorderMode::ALL {
            assert_eq!(BorderMode::from_key(m.key()), Some(m));
        }
        for m in [ScaleMode::Integer, ScaleMode::Fit] {
            assert_eq!(ScaleMode::from_key(m.key()), Some(m));
        }
        assert_eq!(BorderMode::from_key("x"), None);
        assert_eq!(
            window_size_for(BorderMode::Medium.region(), 2, 50.0),
            (640.0, 530.0)
        );
        assert_eq!(scale_label(3.0), "3×");
        assert_eq!(scale_label(2.5), "2.50×");
    }
}
