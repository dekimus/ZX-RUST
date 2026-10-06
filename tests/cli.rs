//! Pruebas de la CLI headless (modo CI): argumentos, determinismo, depurador, trazas y volcados.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_zx48"))
}

fn have_rom() -> bool {
    std::path::Path::new("48.rom").exists()
}

fn run(args: &[&str]) -> Output {
    bin().args(args).output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join("zx48-cli-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().to_string()
}

#[test]
fn usage_and_argument_errors() {
    let o = run(&[]);
    assert_eq!(o.status.code(), Some(1)); // falta --rom
    assert!(String::from_utf8_lossy(&o.stderr).contains("falta --rom"));
    let o = run(&["--bogus"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("uso: zx48"));
    let o = run(&["--help"]);
    assert_eq!(o.status.code(), Some(0));
    let o = run(&["--rom", "48.rom", "--frames", "abc"]);
    assert_eq!(o.status.code(), Some(2));
    let o = run(&["--rom", "no-existe.rom"]);
    assert_eq!(o.status.code(), Some(1));
    let o = run(&["--rom"]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn bad_rom_is_rejected_unless_allowed() {
    let rom = tmp("zero.rom");
    std::fs::write(&rom, vec![0u8; 16384]).unwrap();
    let o = run(&["--rom", &rom]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("SHA-1"));
    let o = run(&[
        "--rom",
        &rom,
        "--allow-nonstandard-rom",
        "--frames",
        "2",
        "--headless",
    ]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("frame=2"));
    let short = tmp("short.rom");
    std::fs::write(&short, [0u8; 100]).unwrap();
    assert_eq!(run(&["--rom", &short]).status.code(), Some(1));
}

#[test]
fn headless_run_is_deterministic_with_golden_hashes() {
    if !have_rom() {
        return;
    }
    let a = stdout(&run(&[
        "--rom",
        "48.rom",
        "--frames",
        "100",
        "--hash",
        "--headless",
    ]));
    let b = stdout(&run(&["--rom", "48.rom", "--frames", "100", "--hash"]));
    assert_eq!(a, b);
    // Valores de referencia del modelo de temporización actual (ROM estándar). Si cambian
    // sin que se haya modificado a propósito CPU/ULA/contención, hay una regresión.
    assert!(a.contains("tstate=6988811 frame=100 pc=0x10AC"), "{a}");
    assert!(
        a.contains("state=fdf0c52ff07583f3 framebuffer=b7ec1b8d225118a2"),
        "{a}"
    );
}

#[test]
fn dumps_have_valid_formats() {
    if !have_rom() {
        return;
    }
    let (scr, bmp, ppm, z80, sna) = (
        tmp("a.scr"),
        tmp("a.bmp"),
        tmp("a.ppm"),
        tmp("a.z80"),
        tmp("a.sna"),
    );
    let o = run(&[
        "--rom",
        "48.rom",
        "--frames",
        "120",
        "--dump-screen",
        &scr,
        "--dump-bmp",
        &bmp,
        "--dump-ppm",
        &ppm,
        "--save-z80",
        &z80,
        "--save-sna",
        &sna,
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(std::fs::metadata(&scr).unwrap().len(), 6912);
    let b = std::fs::read(&bmp).unwrap();
    assert_eq!(&b[..2], b"BM");
    assert_eq!(b.len(), 54 + 352 * 3 * 296);
    assert!(
        std::fs::read(&ppm)
            .unwrap()
            .starts_with(b"P6\n352 296\n255\n")
    );
    assert_eq!(std::fs::metadata(&sna).unwrap().len(), 49179);
    // Los snapshots generados se cargan y continúan donde estaban.
    for f in [&z80, &sna] {
        let o = run(&["--rom", "48.rom", "--snapshot", f, "--tstates", "0"]);
        assert!(
            o.status.success(),
            "{f}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(stdout(&o).contains("pc=0x"), "{}", stdout(&o));
    }
    // Un snapshot corrupto da error de usuario, no un pánico.
    let bad = tmp("bad.z80");
    std::fs::write(&bad, [1u8, 2, 3]).unwrap();
    let o = run(&["--rom", "48.rom", "--snapshot", &bad]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("snapshot inválido"));
}

#[test]
fn cpu_trace_goes_to_stderr_with_disassembly() {
    if !have_rom() {
        return;
    }
    let o = run(&["--rom", "48.rom", "--tstates", "30", "--trace-cpu"]);
    assert!(o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    let first = err.lines().next().unwrap();
    assert!(
        first.contains("zx48::cpu")
            && first.contains("tstate=0")
            && first.contains("pc=0000")
            && first.contains("instr=DI"),
        "{first}"
    );
    assert!(err.contains("instr=XOR A") && err.contains("instr=LD DE,0xFFFF"));
    // Sin flags de traza no hay salida de traza.
    let o = run(&["--rom", "48.rom", "--tstates", "30"]);
    assert!(String::from_utf8_lossy(&o.stderr).is_empty());
}

#[test]
fn ula_trace_reports_frame_events_and_file_output() {
    if !have_rom() {
        return;
    }
    let file = tmp("trace.log");
    let o = run(&[
        "--rom",
        "48.rom",
        "--frames",
        "2",
        "--trace-ula",
        "--trace-file",
        &file,
    ]);
    assert!(o.status.success());
    let log = std::fs::read_to_string(&file).unwrap();
    assert!(
        log.contains("tstate=14336 event=\"FRAME_VISIBLE_START\""),
        "{log}"
    );
    assert!(log.contains("event=\"FRAME_END\" frame=0"));
    assert!(log.contains("tstate=69888 event=\"INT_START\""));
    assert!(String::from_utf8_lossy(&o.stderr).is_empty());
}

#[test]
fn io_trace_shows_border_port_writes() {
    if !have_rom() {
        return;
    }
    let o = run(&["--rom", "48.rom", "--tstates", "100", "--trace-io"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("event=\"OUT\" port=07FE value=07"), "{err}");
}

#[test]
fn interactive_debugger_over_pipes() {
    if !have_rom() {
        return;
    }
    let mut child = bin()
        .args(["--rom", "48.rom", "--debug"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"break 0x11CB\nc 5\nregs\nstep 2\nfrobnicate\nquit\n")
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(out.contains("Breakpoint en 11CB"), "{out}");
    assert!(out.contains("PC=11CB"));
    assert!(out.contains("LD B,A") && out.contains("LD A,0x07"));
    assert!(out.contains("error: comando desconocido: frobnicate"));
}

#[test]
fn debugger_stops_cleanly_on_eof() {
    if !have_rom() {
        return;
    }
    let mut child = bin()
        .args(["--rom", "48.rom", "--debug"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}

#[test]
fn profile_and_bench_report() {
    if !have_rom() {
        return;
    }
    let o = run(&["--rom", "48.rom", "--frames", "30", "--profile", "--bench"]);
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(
        out.contains("bench:") && out.contains("tiempo real = x1.0"),
        "{out}"
    );
    assert!(
        out.contains("Por instrucción:") && out.contains("Por página de 256 bytes:"),
        "{out}"
    );
}

#[test]
fn tape_autoplay_loads_via_cli_options() {
    if !have_rom() {
        return;
    }
    // Con una cinta insertada el programa sigue siendo determinista.
    let a = stdout(&run(&[
        "--rom",
        "48.rom",
        "--tape",
        "tests/data/demo.tap",
        "--frames",
        "30",
        "--hash",
    ]));
    let b = stdout(&run(&[
        "--rom",
        "48.rom",
        "--tape",
        "tests/data/demo.tap",
        "--frames",
        "30",
        "--hash",
    ]));
    assert_eq!(a, b);
    let o = run(&["--rom", "48.rom", "--tape", "no-existe.tap"]);
    assert_eq!(o.status.code(), Some(1));
    let o = run(&["--rom", "48.rom", "--tape", "src/main.rs"]);
    assert_eq!(o.status.code(), Some(1));
}
