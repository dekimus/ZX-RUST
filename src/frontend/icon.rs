//! Icono de la aplicación.
//!
//! `assets/icon.png` (256×256, esquinas con alfa real) se genera a partir de `ico.jpeg`, el
//! asset original del repositorio: el JPEG trae dibujado un damero gris en las esquinas —la
//! típica "transparencia" falsa—, que aquí se sustituye por transparencia auténtica para que
//! el icono se vea limpio en el escritorio. No se lee ningún fichero en tiempo de ejecución:
//! el PNG va embebido en el binario.
//!
//! Regenerar el PNG (solo si cambia `ico.jpeg`): `python3 scripts/make_icon.py`
//! —requiere Pillow y numpy—. El procedimiento está documentado en `docs/UI.md`.

use eframe::egui::{ColorImage, IconData};

/// PNG embebido en el binario.
pub const ICON_PNG: &[u8] = include_bytes!("../../assets/icon.png");

/// Lado del icono en píxeles (múltiplo de 4, como exige `egui::IconData`).
pub const ICON_SIZE: usize = 256;

/// Decodifica el PNG embebido.
///
/// Devuelve `None` si el asset estuviera ausente o corrupto: la ventana se abre con el icono
/// por defecto en lugar de fallar (no se usa `unwrap` sobre contenido del repositorio).
pub fn window_icon() -> Option<IconData> {
    eframe::icon_data::from_png_bytes(ICON_PNG).ok()
}

/// La misma imagen como [`ColorImage`], para mostrarla dentro de la ventana «Acerca de».
pub fn color_image() -> Option<ColorImage> {
    let icon = window_icon()?;
    Some(ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize],
        &icon.rgba,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icono_png_valido_y_del_tamano_esperado() {
        let icon = window_icon().expect("assets/icon.png debe decodificarse");
        assert_eq!(icon.width, ICON_SIZE as u32);
        assert_eq!(icon.height, ICON_SIZE as u32);
        assert_eq!(icon.rgba.len(), ICON_SIZE * ICON_SIZE * 4);
        // El ancho y la altura deben ser múltiplos de 4 (requisito de egui/winit).
        assert_eq!(icon.width % 4, 0);
        assert_eq!(icon.height % 4, 0);
    }

    #[test]
    fn esquinas_transparentes_y_centro_opaco() {
        let icon = window_icon().expect("icono");
        let rgba = &icon.rgba;
        let idx = |x: usize, y: usize| (y * ICON_SIZE + x) * 4;
        // Esquina superior izquierda fuera del logo redondeado: alfa 0.
        assert_eq!(rgba[idx(0, 0) + 3], 0);
        assert_eq!(rgba[idx(ICON_SIZE - 1, 0) + 3], 0);
        assert_eq!(rgba[idx(0, ICON_SIZE - 1) + 3], 0);
        // Centro opaco (el logo es una imagen fotográfica, sin píxeles vacíos).
        let c = idx(ICON_SIZE / 2, ICON_SIZE / 2);
        assert_eq!(rgba[c + 3], 255);
        let brightness: usize = rgba[c..c + 3].iter().map(|&p| usize::from(p)).sum();
        assert!(
            brightness > 60,
            "el centro no puede ser negro puro (suma {brightness})"
        );
    }

    #[test]
    fn color_image_con_las_mismas_dimensiones() {
        let image = color_image().expect("ColorImage");
        assert_eq!(image.size, [ICON_SIZE, ICON_SIZE]);
        assert_eq!(image.pixels.len(), ICON_SIZE * ICON_SIZE);
    }
}
