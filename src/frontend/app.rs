//! Aplicación gráfica: orquesta la ventana, el vídeo, el input y el audio alrededor de `Spectrum48`.
//!
//! Esta capa NO conoce Z80, ULA ni timing: solo usa la API pública del core y decide cuántos
//! fotogramas ejecutar según el reloj del sistema. El reloj interno del Spectrum (T-states) no
//! se toca nunca desde aquí; la sincronización con tiempo real se hace limitando cuántos
//! fotogramas de `69888` T se ejecutan por segundo.

use super::actions::{self, Action};
use super::audio::{self, AudioOutput, AudioProducer};
use super::keymap::{self, Held};
use super::settings::Settings;
use super::viewport::{self, ScaleMode};
use crate::audio::beeper::Resampler;
use crate::machine::spectrum48::Spectrum48;
use crate::ula::{FB_HEIGHT, FB_WIDTH};
use eframe::egui::{self, vec2};
use std::time::{Duration, Instant};

/// Frecuencia nominal de la CPU del Spectrum 48K.
pub const CPU_HZ: u32 = 3_500_000;
/// Duración exacta de un fotograma del Spectrum 48K (69888 T a 3,5 MHz).
pub const FRAME_TIME: Duration = Duration::from_micros(19_968);
/// Fotogramas máximo por actualización de la interfaz (evita la "espiral de la muerte").
pub const MAX_FRAMES_PER_UPDATE: u32 = 5;
/// La ventana de tiempo considerada creíble al recuperarse de una pausa del sistema.
const MAX_CATCHUP: Duration = Duration::from_millis(250);

/// Fotogramas a ejecutar con la acumulación actual. Devuelve (n, deuda restante).
///
/// La deuda nunca crece sin límite: al superar `MAX_FRAMES_PER_UPDATE` se descarta, para que
/// una máquina lenta ralentice la emulación en lugar de acumular retraso.
pub fn frames_to_run(acc: Duration, max: u32) -> (u32, Duration) {
    let mut n = 0;
    let mut rest = acc;
    while rest >= FRAME_TIME && n < max {
        rest -= FRAME_TIME;
        n += 1;
    }
    // Deuda superior al presupuesto: se descarta para no acumular retraso sin fin
    // (solo si el límite estaba activo; con `max == 0` el tiempo se conserva).
    if n == max && max > 0 && rest >= FRAME_TIME {
        rest = Duration::ZERO;
    }
    (n, rest)
}

/// Texto de estado de la cinta para la barra de estado (`--` si no hay cinta).
pub fn tape_status(name: Option<&str>, tape: &crate::tape::TapePlayer) -> String {
    if !tape.has_tape() {
        return "--".to_string();
    }
    let file = name.unwrap_or("cinta");
    if tape.at_end() {
        return format!("{file} · fin");
    }
    let total = tape.block_count();
    let block = tape.block_index() + 1;
    let state = if tape.is_playing() { "PLAY" } else { "STOP" };
    if total > 0 {
        format!("{file} · bloque {block}/{total} · {state}")
    } else {
        format!("{file} · {state}")
    }
}

/// Aplica un diff de entradas (matriz de teclado y joystick Kempston) a la máquina.
fn apply_diff(machine: &mut Spectrum48, d: &keymap::Diff) {
    for &k in &d.key_up {
        machine.key_up(k);
    }
    for &k in &d.key_down {
        machine.key_down(k);
    }
    for &b in &d.joy_up {
        machine.joy_up(b);
    }
    for &b in &d.joy_down {
        machine.joy_down(b);
    }
}

/// Suelta en la máquina todas las entradas (teclado y joystick Kempston) y olvida el estado de
/// la interfaz.
///
/// Se usa siempre que se descarta `held` (carga de snapshot, reset, ROM nueva). Ni el snapshot ni
/// la interfaz "ven" el estado real de entrada: si se olvidara `held` sin avisar a la máquina, una
/// tecla o un botón del Kempston quedarían pulsados para siempre (p. ej. al cargar un `.sna` con
/// el joystick conectado). Por eso se limpia la máquina directamente y no solo el diff.
fn release_inputs(machine: &mut Spectrum48, held: &mut Held) {
    machine.bus.keyboard.release_all();
    machine.bus.kempston.release_all();
    *held = Held::default();
}

pub struct App {
    machine: Spectrum48,
    settings: Settings,
    /// Dispositivo de audio (debe vivir en el hilo de la interfaz).
    audio: Option<AudioOutput>,
    producer: Option<AudioProducer>,
    speaker_resampler: Option<Resampler>,
    tape_resampler: Option<Resampler>,
    /// Búferes reutilizados entre frames (sin allocations en el bucle).
    speaker_buf: Vec<f32>,
    tape_buf: Vec<f32>,
    mixed_buf: Vec<f32>,
    texture: Option<egui::TextureHandle>,
    texture_frame: u64,
    /// Teclas Spectrum/Kempston actualmente pulsadas según la interfaz.
    held: Held,
    paused: bool,
    fullscreen: bool,
    pending: Vec<Action>,
    /// Último atajo despachado (para filtrar rebotes de auto-repetición del sistema).
    last_shortcut: Option<(Action, Instant)>,
    tape_name: Option<String>,
    status: Option<(String, Instant)>,
    error: Option<String>,
    show_help: bool,
    show_about: bool,
    show_debug: bool,
    /// Reloj de sincronización con tiempo real.
    last_tick: Instant,
    acc: Duration,
    /// Medición de FPS (interfaz) y de emulación (fotogramas/segundo).
    stats_t: Instant,
    ui_frames: u32,
    emu_frames: u32,
    fps: f32,
    emu_fps: f32,
    /// Última escala efectiva calculada al dibujar la pantalla (para la barra de estado).
    last_scale_value: f32,
}

impl App {
    /// `tape_name`: nombre del fichero de cinta ya insertado por el arranque (opcional).
    pub fn new(
        mut machine: Spectrum48,
        settings: Settings,
        tape_name: Option<String>,
        initial_error: Option<String>,
    ) -> Self {
        machine.set_kempston(settings.kempston);
        machine.set_tape_autoplay(settings.tape_autoplay);
        let show_debug = settings.show_debug;
        let mut app = Self {
            machine,
            settings,
            audio: None,
            producer: None,
            speaker_resampler: None,
            tape_resampler: None,
            speaker_buf: Vec::new(),
            tape_buf: Vec::new(),
            mixed_buf: Vec::new(),
            texture: None,
            texture_frame: u64::MAX,
            held: Held::default(),
            paused: false,
            fullscreen: false,
            pending: Vec::new(),
            last_shortcut: None,
            tape_name,
            status: None,
            error: initial_error,
            show_help: false,
            show_about: false,
            show_debug,
            last_tick: Instant::now(),
            acc: Duration::ZERO,
            stats_t: Instant::now(),
            ui_frames: 0,
            emu_frames: 0,
            fps: 0.0,
            emu_fps: 0.0,
            last_scale_value: 1.0,
        };
        app.open_audio();
        app
    }

    fn open_audio(&mut self) {
        match AudioOutput::open(self.settings.volume, self.settings.muted) {
            Ok((out, prod)) => {
                let rate = out.sample_rate;
                self.speaker_resampler = Some(Resampler::new(CPU_HZ, rate));
                self.tape_resampler = Some(Resampler::new(CPU_HZ, rate));
                self.producer = Some(prod);
                self.audio = Some(out);
            }
            Err(e) => {
                tracing::warn!(target: "zx48::audio", "sin audio: {e}");
                self.set_status(format!("Audio no disponible: {e}"));
            }
        }
    }

    fn set_status(&mut self, msg: String) {
        self.status = Some((msg, Instant::now()));
    }

    /// Aplica a la máquina las preferencias que la interfaz puede cambiar en caliente.
    fn apply_settings(&mut self) {
        self.machine.set_kempston(self.settings.kempston);
        self.machine.set_tape_autoplay(self.settings.tape_autoplay);
        if let Some(a) = &self.audio {
            a.control.set_volume(self.settings.volume);
            a.control.set_muted(self.settings.muted);
        }
    }

    /// Reajusta el remuestreador tras un salto de reloj (reset, snapshot) o una pausa larga.
    fn resync_audio(&mut self) {
        let t = self.machine.tstate();
        let level = self.machine.bus.beeper.level();
        let tape_level = self.machine.bus.tape.ear_level(t);
        if let Some(r) = &mut self.speaker_resampler {
            r.skip_to(t, level);
        }
        if let Some(r) = &mut self.tape_resampler {
            r.skip_to(t, tape_level);
        }
        // La ventana de tiempo empieza de cero: nada de "deuda" acumulada.
        self.acc = Duration::ZERO;
        self.last_tick = Instant::now();
    }

    /// Vuelca los eventos de beeper/cinta a muestras y las envía al dispositivo (nunca bloquea).
    fn push_audio(&mut self) {
        let speaker = self.machine.drain_audio_events();
        let tape_edges = self.machine.drain_tape_audio_events();
        if self.producer.is_none() {
            return; // Sin dispositivo: ya se drenaron para no acumular eventos.
        }
        let t = self.machine.tstate();
        let Self {
            producer,
            speaker_resampler,
            tape_resampler,
            speaker_buf,
            tape_buf,
            mixed_buf,
            settings,
            ..
        } = self;
        speaker_buf.clear();
        tape_buf.clear();
        if let Some(r) = speaker_resampler.as_mut() {
            r.process(&speaker, t, speaker_buf);
        }
        if let Some(r) = tape_resampler.as_mut() {
            r.process(&tape_edges, t, tape_buf);
        }
        audio::mix(mixed_buf, speaker_buf, tape_buf, settings.tape_sound);
        if let Some(p) = producer.as_mut() {
            p.push(mixed_buf);
        }
    }

    /// Ejecuta emulación según el reloj de pared (solo si no está en pausa).
    fn tick_emulation(&mut self, now: Instant) {
        if self.paused {
            self.last_tick = now;
            return;
        }
        let dt = now
            .saturating_duration_since(self.last_tick)
            .min(MAX_CATCHUP);
        self.last_tick = now;
        self.acc += dt;
        let (n, rest) = frames_to_run(self.acc, MAX_FRAMES_PER_UPDATE);
        self.acc = rest;
        for _ in 0..n {
            self.machine.run_frame();
            self.push_audio();
            self.emu_frames += 1;
        }
    }

    /// Actualiza la textura solo cuando hay un frame nuevo (evita recopiar sin necesidad).
    fn update_texture(&mut self, ctx: &egui::Context) {
        let fc = self.machine.frame_count();
        if fc == self.texture_frame {
            return;
        }
        let opts = if self.settings.linear_filter {
            egui::TextureOptions::LINEAR
        } else {
            egui::TextureOptions::NEAREST
        };
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [FB_WIDTH, FB_HEIGHT],
            self.machine.framebuffer(),
        );
        match &mut self.texture {
            Some(tex) => tex.set(image, opts),
            None => self.texture = Some(ctx.load_texture("zx48-framebuffer", image, opts)),
        }
        self.texture_frame = fc;
    }

    // ------------------------------------------------------------------ acciones

    fn dispatch(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::LoadTape => self.dialog_load_tape(),
            Action::LoadSnapshot => self.dialog_load_snapshot(),
            Action::SaveSnapshot => self.dialog_save_sna(),
            Action::SaveScreenshot => self.dialog_screenshot(),
            Action::SelectRom => self.dialog_select_rom(),
            Action::Reset => {
                self.machine.reset(); // el reset del core ya suelta teclado y joystick
                release_inputs(&mut self.machine, &mut self.held);
                self.resync_audio();
                self.set_status("Máquina reiniciada".into());
            }
            Action::TogglePause => {
                self.paused = !self.paused;
                if !self.paused {
                    self.resync_audio();
                }
            }
            Action::StepFrame => {
                self.paused = true;
                self.machine.run_frame();
                self.push_audio();
                self.emu_frames += 1;
                self.update_texture(ctx);
            }
            Action::TapePlayStop => {
                if self.machine.bus.tape.is_playing() {
                    self.machine.tape_stop();
                    self.set_status("Cinta: STOP".into());
                } else if self.machine.bus.tape.has_tape() {
                    self.machine.tape_play();
                    self.set_status("Cinta: PLAY".into());
                }
            }
            Action::TapeRewind => {
                self.machine.tape_rewind();
                self.set_status("Cinta: rebobinada".into());
            }
            Action::TapeEject => {
                // El core expone la cinta en el bus: expulsar deja el puerto EAR en reposo.
                self.machine.bus.tape.eject();
                self.tape_name = None;
                self.set_status("Cinta expulsada".into());
            }
            Action::ToggleMute => {
                self.settings.muted = !self.settings.muted;
                self.set_status(if self.settings.muted {
                    "Silencio activado".into()
                } else {
                    "Silencio desactivado".into()
                });
            }
            Action::VolumeUp => self.change_volume(0.1),
            Action::VolumeDown => self.change_volume(-0.1),
            Action::ToggleFullscreen => self.toggle_fullscreen(ctx),
            Action::ToggleDebug => {
                self.show_debug = !self.show_debug;
                self.settings.show_debug = self.show_debug;
            }
            Action::ShowHelp => self.show_help = !self.show_help,
            Action::ShowAbout => self.show_about = !self.show_about,
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn change_volume(&mut self, delta: f32) {
        self.settings.volume = (self.settings.volume + delta).clamp(0.0, 1.0);
        if self.settings.volume > 0.0 {
            self.settings.muted = false;
        }
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    fn pick_file(
        title: &str,
        filters: &[&str],
        start: Option<&std::path::Path>,
    ) -> Option<std::path::PathBuf> {
        let mut d = rfd::FileDialog::new().set_title(title);
        if !filters.is_empty() {
            d = d.add_filter("Archivos", filters);
        }
        if let Some(dir) = start {
            d = d.set_directory(dir);
        }
        d.pick_file()
    }

    fn remember_dir(&mut self, path: &std::path::Path) {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                self.settings.last_dir = Some(dir.to_path_buf());
            }
        }
    }

    fn dialog_load_tape(&mut self) {
        let start = self.settings.last_dir.clone();
        let Some(path) = Self::pick_file("Cargar cinta", &["tap", "tzx"], start.as_deref()) else {
            return;
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        match std::fs::read(&path) {
            Err(e) => {
                self.error = Some(format!(
                    "No se pudo leer el fichero:\n{}\n\n{e}",
                    path.display()
                ))
            }
            Ok(data) => match crate::tape::load(&ext, &data) {
                Err(e) => {
                    self.error = Some(format!(
                        "No se pudo cargar la cinta:\n{}\n\n{e}",
                        path.display()
                    ))
                }
                Ok(playable) => {
                    self.machine.insert_tape(playable);
                    self.tape_name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(str::to_string);
                    self.set_status(format!(
                        "Cinta cargada: {}",
                        self.tape_name.as_deref().unwrap_or("?")
                    ));
                }
            },
        }
        self.remember_dir(&path);
    }

    fn dialog_load_snapshot(&mut self) {
        let start = self.settings.last_dir.clone();
        let Some(path) =
            Self::pick_file("Cargar snapshot", &["sna", "z80", "scr"], start.as_deref())
        else {
            return;
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        match std::fs::read(&path) {
            Err(e) => {
                self.error = Some(format!(
                    "No se pudo leer el fichero:\n{}\n\n{e}",
                    path.display()
                ))
            }
            Ok(data) => match self.machine.load_file(&ext, &data) {
                Err(e) => {
                    self.error = Some(format!(
                        "No se pudo cargar el snapshot:\n{}\n\n{e}",
                        path.display()
                    ))
                }
                Ok(()) => {
                    // El snapshot no incluye teclado ni joystick: hay que soltarlos en la
                    // máquina antes de olvidar `held`, si no un botón quedaría pulsado.
                    release_inputs(&mut self.machine, &mut self.held);
                    self.resync_audio();
                    self.set_status(format!("Snapshot cargado: {}", path.display()));
                }
            },
        }
        self.remember_dir(&path);
    }

    fn dialog_save_sna(&mut self) {
        let start = self.settings.last_dir.clone();
        let mut d = rfd::FileDialog::new()
            .set_title("Guardar snapshot SNA")
            .set_file_name("snapshot.sna");
        if let Some(dir) = start.as_deref() {
            d = d.set_directory(dir);
        }
        let Some(path) = d.save_file() else { return };
        match self.machine.save_sna() {
            Err(e) => self.error = Some(format!("No se pudo generar el snapshot:\n{e}")),
            Ok(bytes) => match std::fs::write(&path, &bytes) {
                Err(e) => {
                    self.error = Some(format!(
                        "No se pudo escribir el fichero:\n{}\n\n{e}",
                        path.display()
                    ))
                }
                Ok(()) => self.set_status(format!("Snapshot guardado: {}", path.display())),
            },
        }
        self.remember_dir(&path);
    }

    fn dialog_screenshot(&mut self) {
        let start = self.settings.last_dir.clone();
        let mut d = rfd::FileDialog::new()
            .set_title("Guardar captura (BMP)")
            .set_file_name("pantalla.bmp");
        if let Some(dir) = start.as_deref() {
            d = d.set_directory(dir);
        }
        let Some(path) = d.save_file() else { return };
        let bmp = crate::ula::image::to_bmp(self.machine.framebuffer());
        match std::fs::write(&path, &bmp) {
            Err(e) => {
                self.error = Some(format!(
                    "No se pudo escribir el fichero:\n{}\n\n{e}",
                    path.display()
                ))
            }
            Ok(()) => self.set_status(format!("Captura guardada: {}", path.display())),
        }
        self.remember_dir(&path);
    }

    fn dialog_select_rom(&mut self) {
        let start = self
            .settings
            .rom_path
            .clone()
            .or_else(|| self.settings.last_dir.clone());
        let Some(path) = Self::pick_file(
            "Seleccionar ROM (16 KiB)",
            &["rom", "bin"],
            start.as_deref(),
        ) else {
            return;
        };
        match crate::rom::loader::load_rom(&path, self.settings.allow_nonstandard_rom) {
            Err(e) => {
                let hint = if matches!(e, crate::error::EmulatorError::RomChecksum { .. }) {
                    "\n\nSi es una ROM intencionalmente distinta, actívalo en Archivo → Permitir ROM no estándar."
                } else {
                    ""
                };
                self.error = Some(format!(
                    "No se pudo cargar la ROM:\n{}\n\n{e}{hint}",
                    path.display()
                ));
            }
            Ok(rom) => {
                release_inputs(&mut self.machine, &mut self.held);
                self.machine = Spectrum48::new(rom);
                self.apply_settings();
                self.tape_name = None;
                self.texture_frame = u64::MAX; // la nueva máquina debe refrescarse
                self.resync_audio();
                self.settings.rom_path = Some(path.clone());
                self.set_status(format!(
                    "ROM cargada: {} (máquina reiniciada)",
                    path.display()
                ));
            }
        }
        self.remember_dir(&path);
    }

    // ------------------------------------------------------------------ input

    /// Traduce el teclado del sistema a acciones de la aplicación y a teclas del Spectrum.
    fn handle_input(&mut self, ui: &egui::Ui) {
        let ctx = ui.ctx().clone();
        let (events, focused) = ui.input(|i| (i.events.clone(), i.focused));
        let mut leave_fullscreen = false;
        for e in &events {
            if let egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = e
            {
                if !*pressed || *repeat {
                    continue;
                }
                if let Some(action) = actions::shortcut(*key, *modifiers) {
                    self.pending.push(action);
                } else if *key == egui::Key::Escape && self.fullscreen {
                    leave_fullscreen = true;
                }
            }
        }
        if leave_fullscreen {
            self.fullscreen = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        // Sin foco (o con un campo de texto de la interfaz activo) se libera todo el teclado
        // Spectrum: nada de teclas pulsadas "fantasma" tras cambiar de ventana.
        let te_focused = ctx.text_edit_focused();
        let next = if !focused || te_focused {
            Held::default()
        } else {
            ui.input(|i| keymap::held_from(&i.keys_down, i.modifiers, self.settings.kempston))
        };
        let d = keymap::diff(&self.held, &next);
        apply_diff(&mut self.machine, &d);
        self.held = next;
    }

    // ------------------------------------------------------------------ dibujo

    fn draw_screen(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        let region = self.settings.border_mode.region();
        let p = viewport::place(
            avail.x,
            avail.y,
            region.w,
            region.h,
            self.settings.scale_mode,
        );
        self.last_scale_value = p.scale;
        let (rect, _) = ui.allocate_exact_size(avail.max(vec2(1.0, 1.0)), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let img_min = rect.min + egui::vec2(p.x, p.y);
        let img_rect = egui::Rect::from_min_size(img_min, egui::vec2(p.w.max(1.0), p.h.max(1.0)));
        match &self.texture {
            Some(tex) => {
                let uv = region.uv();
                let uv_rect =
                    egui::Rect::from_min_max(egui::pos2(uv[0], uv[1]), egui::pos2(uv[2], uv[3]));
                painter.image(tex.id(), img_rect, uv_rect, egui::Color32::WHITE);
            }
            None => {
                painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Cargando…",
                    egui::FontId::proportional(20.0),
                    egui::Color32::WHITE,
                );
            }
        }
        if self.paused {
            painter.text(
                img_rect.center(),
                egui::Align2::CENTER_CENTER,
                "PAUSA",
                egui::FontId::proportional((img_rect.height() / 8.0).clamp(14.0, 48.0)),
                egui::Color32::from_white_alpha(230),
            );
        }
    }

    fn draw_menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Archivo", |ui| {
                if menu_action(
                    ui,
                    "Cargar cinta…",
                    actions::shortcut_label(Action::LoadTape),
                ) {
                    self.pending.push(Action::LoadTape);
                }
                if menu_action(
                    ui,
                    "Cargar snapshot…",
                    actions::shortcut_label(Action::LoadSnapshot),
                ) {
                    self.pending.push(Action::LoadSnapshot);
                }
                if menu_action(
                    ui,
                    "Guardar snapshot SNA…",
                    actions::shortcut_label(Action::SaveSnapshot),
                ) {
                    self.pending.push(Action::SaveSnapshot);
                }
                if menu_action(
                    ui,
                    "Guardar captura BMP…",
                    actions::shortcut_label(Action::SaveScreenshot),
                ) {
                    self.pending.push(Action::SaveScreenshot);
                }
                ui.separator();
                if menu_action(
                    ui,
                    "Seleccionar ROM…",
                    actions::shortcut_label(Action::SelectRom),
                ) {
                    self.pending.push(Action::SelectRom);
                }
                let mut allow = self.settings.allow_nonstandard_rom;
                if ui
                    .checkbox(&mut allow, "Permitir ROM no estándar")
                    .changed()
                {
                    self.settings.allow_nonstandard_rom = allow;
                }
                ui.separator();
                if menu_action(ui, "Salir", None) {
                    self.pending.push(Action::Quit);
                }
            });
            ui.menu_button("Emulación", |ui| {
                let pause_label = if self.paused { "Reanudar" } else { "Pausar" };
                if menu_action(
                    ui,
                    pause_label,
                    actions::shortcut_label(Action::TogglePause),
                ) {
                    self.pending.push(Action::TogglePause);
                }
                if menu_action(
                    ui,
                    "Paso de frame",
                    actions::shortcut_label(Action::StepFrame),
                ) {
                    self.pending.push(Action::StepFrame);
                }
                if menu_action(ui, "Reiniciar", actions::shortcut_label(Action::Reset)) {
                    self.pending.push(Action::Reset);
                }
                ui.separator();
                ui.menu_button("Cinta", |ui| {
                    if menu_action(
                        ui,
                        "Play / Stop",
                        actions::shortcut_label(Action::TapePlayStop),
                    ) {
                        self.pending.push(Action::TapePlayStop);
                    }
                    if menu_action(ui, "Rebobinar", actions::shortcut_label(Action::TapeRewind)) {
                        self.pending.push(Action::TapeRewind);
                    }
                    if ui.button("Expulsar").clicked() {
                        self.pending.push(Action::TapeEject);
                    }
                });
                ui.separator();
                let mut joy = self.settings.kempston;
                if ui
                    .checkbox(&mut joy, "Joystick Kempston (flechas)")
                    .changed()
                {
                    self.settings.kempston = joy;
                }
            });
            ui.menu_button("Vídeo", |ui| {
                let mut integer = self.settings.scale_mode == ScaleMode::Integer;
                if ui
                    .radio_value(&mut integer, true, "Escalado entero")
                    .changed()
                {
                    self.settings.scale_mode = if integer {
                        ScaleMode::Integer
                    } else {
                        ScaleMode::Fit
                    };
                }
                let mut fit = self.settings.scale_mode == ScaleMode::Fit;
                if ui
                    .radio_value(&mut fit, true, "Ajustar a la ventana")
                    .changed()
                {
                    self.settings.scale_mode = if fit {
                        ScaleMode::Fit
                    } else {
                        ScaleMode::Integer
                    };
                }
                ui.separator();
                for mode in viewport::BorderMode::ALL {
                    let mut selected = self.settings.border_mode == mode;
                    if ui.radio_value(&mut selected, true, mode.label()).changed() {
                        self.settings.border_mode = mode;
                    }
                }
                ui.separator();
                let mut linear = self.settings.linear_filter;
                if ui.checkbox(&mut linear, "Filtro suave").changed() {
                    self.settings.linear_filter = linear;
                }
                if menu_action(
                    ui,
                    "Pantalla completa",
                    actions::shortcut_label(Action::ToggleFullscreen),
                ) {
                    self.pending.push(Action::ToggleFullscreen);
                }
                ui.separator();
                let mut bar = self.settings.show_status_bar;
                if ui.checkbox(&mut bar, "Barra de estado").changed() {
                    self.settings.show_status_bar = bar;
                }
                let mut dbg = self.show_debug;
                if ui.checkbox(&mut dbg, "Información de depuración").changed() {
                    self.show_debug = dbg;
                    self.settings.show_debug = dbg;
                }
            });
            ui.menu_button("Audio", |ui| {
                let mut muted = self.settings.muted;
                if ui.checkbox(&mut muted, "Silenciar").changed() {
                    self.settings.muted = muted;
                }
                ui.add(
                    egui::Slider::new(&mut self.settings.volume, 0.0..=1.0)
                        .fixed_decimals(0)
                        .text("Volumen"),
                );
                let mut tape_sound = self.settings.tape_sound;
                if ui.checkbox(&mut tape_sound, "Sonido de cinta").changed() {
                    self.settings.tape_sound = tape_sound;
                }
            });
            ui.menu_button("Ayuda", |ui| {
                if menu_action(
                    ui,
                    "Atajos de teclado",
                    actions::shortcut_label(Action::ShowHelp),
                ) {
                    self.pending.push(Action::ShowHelp);
                }
                if ui.button("Acerca de…").clicked() {
                    self.pending.push(Action::ShowAbout);
                }
            });
        });
    }

    fn draw_status(&mut self, ui: &mut egui::Ui) {
        let run = if self.paused {
            "● En pausa"
        } else {
            "● Funcionando"
        };
        let tape = tape_status(self.tape_name.as_deref(), &self.machine.bus.tape);
        let vol = if self.settings.muted {
            "mute".to_string()
        } else {
            format!("{}%", (self.settings.volume * 100.0).round() as u32)
        };
        let scale = viewport::scale_label(self.last_scale());
        // Degradación elegante: con una ventana estrecha se ocultan primero los campos menos
        // esenciales para que el grupo de la derecha (mensajes/FPS) nunca se pegue a los otros.
        let width = ui.available_width();
        let show_volume = width >= 430.0;
        let show_scale = width >= 540.0;
        ui.horizontal(|ui| {
            ui.label(run);
            ui.separator();
            ui.label(format!("Cinta: {tape}"));
            if show_volume {
                ui.separator();
                ui.label(format!("Volumen: {vol}"));
            }
            if show_scale {
                ui.separator();
                ui.label(format!("Escala: {scale}"));
            }
            let avail = ui.available_size();
            ui.allocate_ui_with_layout(
                avail,
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    // Mensaje temporal si está fresco; si no, FPS/estado.
                    let fresh = self
                        .status
                        .as_ref()
                        .is_some_and(|(_, t)| t.elapsed() < Duration::from_secs(5));
                    if fresh {
                        if let Some((msg, _)) = &self.status {
                            ui.label(msg.clone());
                        }
                    } else if self.paused {
                        // En pausa el contador de FPS es engañoso (no hay repintados): se
                        // muestra el reloj congelado de la máquina, que es lo que importa.
                        ui.label(format!("T-states: {}", self.machine.tstate()));
                    } else {
                        ui.label(format!("FPS {:.0} · EMU {:.1} Hz", self.fps, self.emu_fps));
                    }
                    // Último elemento en el layout inverso = izquierda del grupo: separa el
                    // bloque derecho de los campos de la izquierda.
                    ui.separator();
                },
            );
        });
    }

    /// Escala actual efectiva (para la barra de estado): se recalcula con el tamaño real.
    fn last_scale(&self) -> f32 {
        self.last_scale_value
    }

    fn draw_windows(&mut self, ctx: &egui::Context) {
        if let Some(err) = self.error.clone() {
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(err);
                    ui.add_space(8.0);
                    if ui.button("Cerrar").clicked() {
                        self.error = None;
                    }
                });
        }
        let mut help = self.show_help;
        egui::Window::new("Atajos de teclado").open(&mut help).resizable(false).show(ctx, |ui| {
            ui.label("Teclas de función y de navegación; el resto del teclado va al Spectrum.");
            ui.add_space(4.0);
            egui::Grid::new("shortcuts").num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                for (_key, action, label) in actions::SHORTCUTS {
                    ui.label(format!("{label:>8}"));
                    ui.label(shortcut_name(*action));
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.label("Sin teclas de función: ESC sale de pantalla completa; el teclado normal (letras, números, flechas, Shift, Ctrl/Alt) es el del Spectrum.");
        });
        self.show_help = help;
        let mut about = self.show_about;
        egui::Window::new("Acerca de").open(&mut about).resizable(false).show(ctx, |ui| {
            ui.heading("zx48");
            ui.label("Emulador de ZX Spectrum 48K en Rust.");
            ui.label("Núcleo independiente de la interfaz: CPU Z80, ULA con contención, cinta TAP/TZX, snapshots SNA/Z80.");
            ui.add_space(4.0);
            ui.label("La ROM se carga de fuera del repositorio (no se distribuye).");
        });
        self.show_about = about;
        let mut dbg = self.show_debug;
        egui::Window::new("Depuración")
            .open(&mut dbg)
            .resizable(false)
            .collapsible(true)
            .show(ctx, |ui| {
                let r = &self.machine.cpu.regs;
                egui::Grid::new("dbg")
                    .num_columns(2)
                    .spacing([16.0, 2.0])
                    .show(ui, |ui| {
                        ui.label("FPS interfaz");
                        ui.label(format!("{:.1}", self.fps));
                        ui.end_row();
                        ui.label("EMU");
                        ui.label(format!("{:.2} Hz", self.emu_fps));
                        ui.end_row();
                        ui.label("T-states");
                        ui.label(format!("{}", self.machine.tstate()));
                        ui.end_row();
                        ui.label("Frame");
                        ui.label(format!("{}", self.machine.frame_count()));
                        ui.end_row();
                        ui.label("PC");
                        ui.label(format!("{:04X}", r.pc));
                        ui.end_row();
                        ui.label("SP");
                        ui.label(format!("{:04X}", r.sp));
                        ui.end_row();
                        if let Some(p) = &self.producer {
                            ui.label("Audio en cola");
                            ui.label(format!("{} muestras", p.fill()));
                            ui.end_row();
                        }
                    });
            });
        self.show_debug = dbg;
    }

    fn update_stats(&mut self) {
        self.ui_frames += 1;
        let elapsed = self.stats_t.elapsed();
        if elapsed >= Duration::from_secs(1) {
            self.fps = self.ui_frames as f32 / elapsed.as_secs_f32();
            self.emu_fps = self.emu_frames as f32 / elapsed.as_secs_f32();
            self.ui_frames = 0;
            self.emu_frames = 0;
            self.stats_t = Instant::now();
        }
    }

    /// Persiste las preferencias al salir de la aplicación.
    pub fn persist(&self) {
        if let Err(e) = self.settings.save() {
            tracing::warn!(target: "zx48", "no se pudo guardar la configuración: {e}");
        }
    }
}

/// Elemento de menú con atajo mostrado a la derecha. Devuelve `true` si se pulsó.
fn menu_action(ui: &mut egui::Ui, label: &str, shortcut: Option<&str>) -> bool {
    let text = match shortcut {
        Some(s) => format!("{label}    {s}"),
        None => label.to_string(),
    };
    ui.button(text).clicked()
}

fn shortcut_name(action: Action) -> &'static str {
    match action {
        Action::LoadTape => "Cargar cinta",
        Action::LoadSnapshot => "Cargar snapshot",
        Action::SaveSnapshot => "Guardar snapshot",
        Action::SaveScreenshot => "Guardar captura",
        Action::SelectRom => "Seleccionar ROM",
        Action::Reset => "Reiniciar",
        Action::TogglePause => "Pausa / reanudar",
        Action::StepFrame => "Paso de frame",
        Action::TapePlayStop => "Cinta play/stop",
        Action::TapeRewind => "Cinta rebobinar",
        Action::TapeEject => "Expulsar cinta",
        Action::ToggleMute => "Silenciar",
        Action::VolumeUp => "Subir volumen",
        Action::VolumeDown => "Bajar volumen",
        Action::ToggleFullscreen => "Pantalla completa",
        Action::ToggleDebug => "Información de depuración",
        Action::ShowHelp => "Atajos",
        Action::ShowAbout => "Acerca de",
        Action::Quit => "Salir",
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // 1) Input → 2) acciones → 3) emulación → 4) vídeo → 5) dibujo.
        self.handle_input(ui);
        let pending: Vec<Action> = std::mem::take(&mut self.pending);
        let now = Instant::now();
        for a in pending {
            if actions::debounced(self.last_shortcut, a, now) {
                continue;
            }
            self.last_shortcut = Some((a, now));
            self.dispatch(a, &ctx);
        }
        self.apply_settings();
        self.tick_emulation(Instant::now());
        self.update_texture(&ctx);
        self.update_stats();
        if !self.paused {
            // La interfaz se repinta cuando toca ejecutar el siguiente fotograma del Spectrum
            // (≤ 50 Hz de pintado); en pausa no hace falta repintar: los eventos del sistema
            // (ratón/teclado) ya provocan repintado.
            //
            // egui resta `predicted_dt` al retardo pedido (quiere disparar antes del siguiente
            // vsync), así que se le pide `wait + predicted_dt` para que el retardo neto sea
            // exactamente el tiempo que falta para el próximo fotograma.
            let wait = FRAME_TIME.saturating_sub(self.acc);
            let predicted_dt = ctx.input(|i| i.predicted_dt);
            let predicted = if predicted_dt.is_finite() && predicted_dt > 0.0 {
                Duration::from_secs_f64(f64::from(predicted_dt))
            } else {
                Duration::ZERO
            };
            ctx.request_repaint_after(wait + predicted);
        }

        egui::Panel::top("zx48_menu")
            .exact_size(26.0)
            .show_separator_line(false)
            .show(ui, |ui| {
                self.draw_menu(ui);
            });
        if self.settings.show_status_bar {
            egui::Panel::bottom("zx48_status")
                .exact_size(22.0)
                .show_separator_line(false)
                .show(ui, |ui| {
                    self.draw_status(ui);
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::from_rgb(8, 8, 12)))
            .show(ui, |ui| {
                self.draw_screen(ui);
            });
        self.draw_windows(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacing_runs_exactly_frames_per_budget() {
        // 3 frames exactos + resto.
        let (n, rest) = frames_to_run(FRAME_TIME * 3, MAX_FRAMES_PER_UPDATE);
        assert_eq!((n, rest), (3, Duration::ZERO));
        let (n, rest) = frames_to_run(
            FRAME_TIME * 3 + Duration::from_millis(5),
            MAX_FRAMES_PER_UPDATE,
        );
        assert_eq!((n, rest), (3, Duration::from_millis(5)));
        // Deuda grande: se ejecuta el máximo y el resto se descarta.
        let (n, rest) = frames_to_run(Duration::from_secs(2), MAX_FRAMES_PER_UPDATE);
        assert_eq!((n, rest), (MAX_FRAMES_PER_UPDATE, Duration::ZERO));
        // Nada pendiente.
        assert_eq!(
            frames_to_run(Duration::ZERO, MAX_FRAMES_PER_UPDATE),
            (0, Duration::ZERO)
        );
        // Límite configurable a cero: no ejecuta nada (y no descarta el tiempo).
        assert_eq!(frames_to_run(FRAME_TIME, 0), (0, FRAME_TIME));
    }

    #[test]
    fn frame_time_matches_the_hardware() {
        assert_eq!(FRAME_TIME.as_micros(), 19_968);
        // 69888 T / 3.500.000 Hz = 0,019968 s exactos.
        assert_eq!(69_888_000_000u64 / 3_500_000, FRAME_TIME.as_micros() as u64);
    }

    /// Teclado y joystick llegan a la máquina por el mismo camino que usa `handle_input`.
    #[test]
    fn apply_diff_sends_presses_and_releases_to_the_machine() {
        use crate::input::kempston::JoyButton;
        use crate::input::keyboard::SpectrumKey;
        use crate::rom::loader::ROM_SIZE;

        let mut m = Spectrum48::new(Box::new([0u8; ROM_SIZE]));
        m.set_kempston(true);
        let pressed = Held {
            spectrum: [SpectrumKey::A].into_iter().collect(),
            joy: [JoyButton::Left].into_iter().collect(),
        };
        let none = Held::default();

        apply_diff(&mut m, &keymap::diff(&none, &pressed));
        assert_eq!(m.bus.kempston.read(), 0x02, "izquierda pulsada");
        assert_eq!(m.bus.keyboard.read(0xFD), 0x1E, "A pulsada en la matriz");

        apply_diff(&mut m, &keymap::diff(&pressed, &none));
        assert_eq!(m.bus.kempston.read(), 0, "izquierda suelta");
        assert_eq!(m.bus.keyboard.read(0xFD), 0x1F, "A suelta en la matriz");
    }

    /// Regresión: cargar un snapshot (`.sna`, `.z80`, `.scr`) descarta el estado de entrada de la
    /// interfaz; si antes no se limpia la máquina, con el joystick conectado un botón se queda
    /// pulsado para siempre.
    #[test]
    fn discarding_held_releases_keyboard_and_joystick() {
        use crate::input::kempston::JoyButton;
        use crate::input::keyboard::SpectrumKey;
        use crate::rom::loader::ROM_SIZE;

        let mut m = Spectrum48::new(Box::new([0u8; ROM_SIZE]));
        m.set_kempston(true);
        let mut held = Held {
            spectrum: [SpectrumKey::Enter].into_iter().collect(),
            joy: [JoyButton::Fire].into_iter().collect(),
        };
        apply_diff(&mut m, &keymap::diff(&Held::default(), &held));
        assert_eq!(m.bus.kempston.read(), 0x10, "disparo en el puerto Kempston");
        assert_eq!(m.bus.keyboard.read(0xBF), 0x1E, "Enter en la matriz");

        // La interfaz olvida `held` al cargar el snapshot…
        release_inputs(&mut m, &mut held);
        // …y la máquina no conserva nada pulsado.
        assert_eq!(m.bus.kempston.read(), 0, "disparo suelto tras la carga");
        assert_eq!(
            m.bus.keyboard.read(0xBF),
            0x1F,
            "Enter suelta tras la carga"
        );
        assert!(held.spectrum.is_empty() && held.joy.is_empty());
    }

    #[test]
    fn tape_status_is_conservative_without_a_tape() {
        let tape = crate::tape::TapePlayer::new();
        assert_eq!(tape_status(None, &tape), "--");
        assert_eq!(tape_status(Some("juego.tap"), &tape), "--");
    }
}
