//! Punto de entrada de la aplicación gráfica (eframe/egui + OpenGL).
//!
//! Uso: `zx48-gui [--allow-nonstandard-rom] [ROM] [archivo]`
//!
//! * `ROM` (opcional): fichero de 16 KiB. Si no se indica, se prueba la ruta guardada en la
//!   configuración y después `./48.rom`; si ninguna existe, se abre un selector de ficheros.
//! * `archivo` (opcional): cinta (`.tap`, `.tzx`) o snapshot (`.sna`, `.z80`, `.scr`) inicial.

use std::path::{Path, PathBuf};
use zx48::frontend::app::App;
use zx48::frontend::icon;
use zx48::frontend::settings::Settings;
use zx48::frontend::viewport;
use zx48::machine::spectrum48::Spectrum48;
use zx48::rom::loader;

struct Args {
    rom: Option<PathBuf>,
    file: Option<PathBuf>,
    allow_nonstandard: bool,
    help: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        rom: None,
        file: None,
        allow_nonstandard: false,
        help: false,
    };
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-h" | "--help" => args.help = true,
            "--allow-nonstandard-rom" => args.allow_nonstandard = true,
            _ if a.starts_with('-') => {
                eprintln!("opción desconocida: {a}\n{}", usage());
                std::process::exit(2);
            }
            _ => {
                if args.rom.is_none() {
                    args.rom = Some(PathBuf::from(a));
                } else if args.file.is_none() {
                    args.file = Some(PathBuf::from(a));
                } else {
                    eprintln!("demasiados argumentos\n{}", usage());
                    std::process::exit(2);
                }
            }
        }
    }
    args
}

fn usage() -> String {
    "Uso: zx48-gui [--allow-nonstandard-rom] [ROM] [archivo]\n\
     \n\
     ROM      48.rom (16 KiB). Si se omite: configuración guardada, luego ./48.rom,\n\
              y si no existe se abre un selector de ficheros.\n\
     archivo  .tap/.tzx (cinta) o .sna/.z80/.scr (snapshot) inicial."
        .to_string()
}

/// Busca la ROM: argumento → configuración → `./48.rom` → selector del usuario.
fn resolve_rom(args: &Args, settings: &Settings) -> Option<PathBuf> {
    // Si el usuario indicó una ROM explícita, se usa aunque no exista: el error será claro.
    if let Some(p) = &args.rom {
        return Some(p.clone());
    }
    let candidates: Vec<PathBuf> = [settings.rom_path.clone(), Some(PathBuf::from("48.rom"))]
        .into_iter()
        .flatten()
        .collect();
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    // Último recurso: preguntar al usuario (selector nativo).
    rfd::FileDialog::new()
        .set_title("Selecciona la ROM del ZX Spectrum 48K (16 KiB)")
        .add_filter("ROM", &["rom", "bin", "48"])
        .pick_file()
}

fn load_machine(path: &Path, allow_nonstandard: bool) -> Result<Spectrum48, String> {
    let rom = loader::load_rom(path, allow_nonstandard)
        .map_err(|e| format!("No se pudo cargar la ROM {}\n\n{e}", path.display()))?;
    Ok(Spectrum48::new(rom))
}

/// Carga el fichero inicial (cinta o snapshot) sin abusar de la ROM: cualquier error se
/// devuelve como mensaje y la máquina queda usable.
fn load_initial(machine: &mut Spectrum48, path: &Path) -> Result<Option<String>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    let data =
        std::fs::read(path).map_err(|e| format!("No se pudo leer {}\n\n{e}", path.display()))?;
    match ext.to_ascii_lowercase().as_str() {
        "tap" | "tzx" => {
            let playable = zx48::tape::load(&ext, &data)
                .map_err(|e| format!("No se pudo cargar la cinta {}\n\n{e}", path.display()))?;
            machine.insert_tape(playable);
            Ok(Some(
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("cinta")
                    .to_string(),
            ))
        }
        "sna" | "z80" | "scr" => {
            machine
                .load_file(&ext, &data)
                .map_err(|e| format!("No se pudo cargar el snapshot {}\n\n{e}", path.display()))?;
            Ok(None)
        }
        other => Err(format!(
            "Formato desconocido: .{other}\nSe admiten tap, tzx, sna, z80 y scr."
        )),
    }
}

fn main() -> eframe::Result<()> {
    let args = parse_args();
    if args.help {
        println!("{}", usage());
        return Ok(());
    }
    let mut settings = Settings::load();
    if args.allow_nonstandard {
        settings.allow_nonstandard_rom = true;
    }

    // 1) ROM: imprescindible para arrancar; sin ella se explica el motivo y se sale.
    let rom_path = match resolve_rom(&args, &settings) {
        Some(p) => p,
        None => {
            eprintln!("No se encontró la ROM de 16 KiB (48.rom).\n{}", usage());
            std::process::exit(1);
        }
    };
    let mut machine = match load_machine(&rom_path, settings.allow_nonstandard_rom) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    settings.rom_path = Some(rom_path);

    // 2) Fichero inicial opcional: los errores no impiden arrancar, se muestran en la interfaz.
    let mut tape_name = None;
    let mut initial_error = None;
    if let Some(file) = &args.file {
        match load_initial(&mut machine, file) {
            Ok(name) => tape_name = name,
            Err(e) => initial_error = Some(e),
        }
    }

    // 3) Ventana: tamaño inicial para mostrar la imagen a escala entera con las barras de UI.
    let region = settings.border_mode.region();
    let (w, h) = viewport::window_size_for(region, settings.window_scale.max(1), UI_CHROME);
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("ZX Spectrum 48K")
        .with_inner_size([w, h])
        .with_min_inner_size([320.0, 240.0]);
    // Icono de la ventana (embebido; si no decodifica, se queda el icono por defecto).
    if let Some(icon) = icon::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "zx48",
        options,
        Box::new(move |_| {
            Ok(Box::new(App::new(
                machine,
                settings,
                tape_name,
                initial_error,
            )))
        }),
    )
}

/// Altura aproximada de menú + barra de estado (para calcular el tamaño de ventana inicial).
const UI_CHROME: f32 = 48.0;
