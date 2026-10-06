//! CLI headless del emulador (CI, depuración, trazas, perfil). Sin ventana: la interfaz
//! gráfica es el binario `zx48-gui` (feature `gui`).

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Instant;
use tracing::Level;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use zx48::debug::Debugger;
use zx48::ula::image;
use zx48::{EmulatorError, Spectrum48, rom::loader};

const USAGE: &str = "\
uso: zx48 --rom 48.rom [opciones]

Máquina:
  --rom <fichero>            ROM de 16 KiB (obligatoria)
  --allow-nonstandard-rom    acepta una ROM con SHA-1 distinto del estándar
  --tape <x.tap|x.tzx>       inserta una cinta (arranca sola al entrar la ROM en LD-BYTES)
  --snapshot <x.sna|x.z80|x.scr>   carga un snapshot o una pantalla
  --kempston                 conecta el joystick Kempston

Ejecución:
  --frames <n>               ejecuta n frames (por defecto 1 si no hay --tstates)
  --tstates <n>              ejecuta al menos n T-states
  --headless                 (compatibilidad) este binario nunca abre ventana
  --debug                    depurador interactivo por stdin/stdout (help dentro)
  --profile                  perfil de instrucciones tras la ejecución
  --bench                    mide la velocidad de emulación respecto al tiempo real

Salida:
  --dump-screen <x.scr>  --dump-ppm <x.ppm>  --dump-bmp <x.bmp>
  --save-z80 <x.z80>     --save-sna <x.sna>
  --hash                     imprime hashes deterministas del estado y del framebuffer

Trazas (a stderr o --trace-file):
  --trace-cpu --trace-ula --trace-io --trace-tape --trace-audio --trace-snapshot --trace-all
  --trace-file <fichero>";

#[derive(Default)]
struct Options {
    rom: Option<PathBuf>,
    allow_nonstandard: bool,
    tape: Option<PathBuf>,
    snapshot: Option<PathBuf>,
    kempston: bool,
    frames: Option<u64>,
    tstates: Option<u64>,
    debug: bool,
    profile: bool,
    bench: bool,
    hash: bool,
    dump_scr: Option<PathBuf>,
    dump_ppm: Option<PathBuf>,
    dump_bmp: Option<PathBuf>,
    save_z80: Option<PathBuf>,
    save_sna: Option<PathBuf>,
    trace: Vec<(&'static str, Level)>,
    trace_file: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    let value = |it: &mut std::slice::Iter<String>, flag: &str| -> Result<String, String> {
        it.next()
            .cloned()
            .ok_or_else(|| format!("{flag} requiere un valor"))
    };
    let number = |s: String, flag: &str| -> Result<u64, String> {
        s.parse()
            .map_err(|_| format!("{flag}: número inválido '{s}'"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--rom" => o.rom = Some(value(&mut it, a)?.into()),
            "--allow-nonstandard-rom" => o.allow_nonstandard = true,
            "--tape" => o.tape = Some(value(&mut it, a)?.into()),
            "--snapshot" => o.snapshot = Some(value(&mut it, a)?.into()),
            "--kempston" => o.kempston = true,
            "--frames" => o.frames = Some(number(value(&mut it, a)?, a)?),
            "--tstates" => o.tstates = Some(number(value(&mut it, a)?, a)?),
            "--headless" => {}
            "--debug" => o.debug = true,
            "--profile" => o.profile = true,
            "--bench" => o.bench = true,
            "--hash" => o.hash = true,
            "--dump-screen" => o.dump_scr = Some(value(&mut it, a)?.into()),
            "--dump-ppm" => o.dump_ppm = Some(value(&mut it, a)?.into()),
            "--dump-bmp" => o.dump_bmp = Some(value(&mut it, a)?.into()),
            "--save-z80" => o.save_z80 = Some(value(&mut it, a)?.into()),
            "--save-sna" => o.save_sna = Some(value(&mut it, a)?.into()),
            "--trace-cpu" => o.trace.push(("zx48::cpu", Level::TRACE)),
            "--trace-ula" => {
                o.trace.push(("zx48::ula", Level::TRACE));
                o.trace.push(("zx48::video", Level::TRACE));
            }
            "--trace-io" => o.trace.push(("zx48::io", Level::TRACE)),
            "--trace-tape" => o.trace.push(("zx48::tape", Level::DEBUG)),
            "--trace-audio" => o.trace.push(("zx48::audio", Level::TRACE)),
            "--trace-snapshot" => o.trace.push(("zx48::snapshot", Level::DEBUG)),
            "--trace-all" => o.trace.push(("zx48", Level::TRACE)),
            "--trace-file" => o.trace_file = Some(value(&mut it, a)?.into()),
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("opción desconocida: {other}")),
        }
    }
    Ok(o)
}

fn init_tracing(o: &Options) -> Result<(), String> {
    if o.trace.is_empty() {
        return Ok(());
    }
    let mut targets = Targets::new();
    for (t, l) in &o.trace {
        targets = targets.with_target(*t, *l);
    }
    let layer = fmt::layer().without_time().with_ansi(false);
    let registry = tracing_subscriber::registry().with(targets);
    match &o.trace_file {
        Some(p) => {
            let f = std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?;
            registry
                .with(layer.with_writer(std::sync::Mutex::new(f)))
                .init();
        }
        None => registry.with(layer.with_writer(std::io::stderr)).init(),
    }
    Ok(())
}

fn write_file(path: &PathBuf, data: &[u8]) -> Result<(), String> {
    std::fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))
}

fn ext(path: &std::path::Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string()
}

fn run(o: &Options) -> Result<(), String> {
    init_tracing(o)?;
    let rom_path = o.rom.as_ref().ok_or("falta --rom")?;
    let rom = loader::load_rom(rom_path, o.allow_nonstandard).map_err(|e| e.to_string())?;
    let mut m = Spectrum48::new(rom);
    m.reset();
    m.set_kempston(o.kempston);
    let fmt_err = |p: &PathBuf, e: EmulatorError| format!("{}: {e}", p.display());
    if let Some(p) = &o.snapshot {
        let data = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
        m.load_file(&ext(p), &data).map_err(|e| fmt_err(p, e))?;
    }
    if let Some(p) = &o.tape {
        let data = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
        m.insert_tape(zx48::tape::load(&ext(p), &data).map_err(|e| fmt_err(p, e))?);
        m.set_tape_autoplay(true);
    }

    let frames = o
        .frames
        .or(if o.tstates.is_none() { Some(1) } else { None });
    let mut debugger = Debugger::new();
    if o.profile {
        debugger.start_profile();
    }
    let t0 = Instant::now();
    let start_t = m.tstate();
    let budget = match (frames, o.tstates) {
        (Some(f), _) => f * zx48::ula::timing::TSTATES_PER_FRAME as u64,
        (None, Some(t)) => t,
        (None, None) => 0,
    };
    if o.debug {
        // En modo depuración la ejecución inicial es opcional (solo si se piden frames/tstates).
        if o.frames.is_some() || o.tstates.is_some() {
            debugger.run(&mut m, budget, u64::MAX);
        }
    } else if o.profile {
        debugger.run(&mut m, budget, u64::MAX);
    } else if let Some(f) = frames {
        (0..f).for_each(|_| m.run_frame());
    } else {
        m.run_tstates(o.tstates.unwrap_or(0));
    }
    let wall = t0.elapsed().as_secs_f64();
    m.sync_video();

    println!(
        "tstate={} frame={} pc={:#06X}",
        m.tstate(),
        m.frame_count(),
        m.cpu.regs.pc
    );
    if o.bench {
        let emulated = (m.tstate() - start_t) as f64 / 3_500_000.0;
        println!(
            "bench: {:.3} s emulados en {:.3} s reales = x{:.1} (tiempo real = x1.0)",
            emulated,
            wall,
            if wall > 0.0 {
                emulated / wall
            } else {
                f64::INFINITY
            }
        );
    }
    if o.hash {
        println!(
            "state={:016x} framebuffer={:016x}",
            m.state_hash(),
            m.framebuffer_hash()
        );
    }
    if o.profile {
        if let Some(r) = debugger.profile_report(&m, 15) {
            println!("{r}");
        }
    }
    if o.debug {
        repl(&mut m, &mut debugger);
    }
    if let Some(p) = &o.dump_scr {
        write_file(p, &m.save_scr())?;
    }
    if let Some(p) = &o.dump_ppm {
        write_file(p, &image::to_ppm(m.framebuffer()))?;
    }
    if let Some(p) = &o.dump_bmp {
        write_file(p, &image::to_bmp(m.framebuffer()))?;
    }
    if let Some(p) = &o.save_z80 {
        write_file(p, &m.save_z80())?;
    }
    if let Some(p) = &o.save_sna {
        write_file(p, &m.save_sna().map_err(|e| fmt_err(p, e))?)?;
    }
    Ok(())
}

fn repl(m: &mut Spectrum48, d: &mut Debugger) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    println!("Depurador ZX Spectrum 48K (help para ver los comandos)");
    loop {
        print!("zx48> ");
        let _ = stdout.flush();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let out = d.execute(m, line.trim());
        if !out.text.is_empty() {
            println!("{}", out.text);
        }
        if out.quit {
            break;
        }
    }
    m.sync_video();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}\n");
            }
            eprintln!("{USAGE}");
            std::process::exit(if e.is_empty() { 0 } else { 2 });
        }
    };
    if let Err(e) = run(&opts) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
