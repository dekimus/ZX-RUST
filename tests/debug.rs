use zx48::Spectrum48;
use zx48::debug::{Debugger, StopReason, WatchKind};

fn machine(program: &[u8]) -> Spectrum48 {
    let mut rom = Box::new([0u8; 16384]);
    rom[..program.len()].copy_from_slice(program);
    let mut m = Spectrum48::new(rom);
    m.reset();
    m
}

fn run(d: &mut Debugger, m: &mut Spectrum48, cmd: &str) -> String {
    d.execute(m, cmd).text
}

// LD A,5 ; LD B,6 ; LD (0x8000),A ; LD A,(0x8000) ; JR $
const PROG: &[u8] = &[
    0x3E, 0x05, 0x06, 0x06, 0x32, 0x00, 0x80, 0x3A, 0x00, 0x80, 0x18, 0xFE,
];

#[test]
fn breakpoint_stops_before_executing_and_continue_moves_on() {
    let mut m = machine(PROG);
    let mut d = Debugger::new();
    assert!(run(&mut d, &mut m, "break 4").contains("0004"));
    let r = d.run(&mut m, 1_000_000, u64::MAX);
    assert_eq!(r, StopReason::Breakpoint(4));
    assert_eq!(m.cpu.regs.pc, 4);
    assert_eq!((m.cpu.regs.a, m.cpu.regs.b), (5, 6));
    // Desde el breakpoint se puede continuar (la primera instrucción siempre se ejecuta).
    d.add_breakpoint(0x0A);
    assert_eq!(
        d.run(&mut m, 1_000_000, u64::MAX),
        StopReason::Breakpoint(0x0A)
    );
    assert_eq!(run(&mut d, &mut m, "breaks"), "0004 000A");
    run(&mut d, &mut m, "delete 4");
    assert_eq!(run(&mut d, &mut m, "breaks"), "000A");
    run(&mut d, &mut m, "delete all");
    assert_eq!(run(&mut d, &mut m, "breaks"), "(sin breakpoints)");
}

#[test]
fn step_prints_disassembly_and_tstates() {
    let mut m = machine(PROG);
    let mut d = Debugger::new();
    let out = run(&mut d, &mut m, "step 3");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(
        lines[0].contains("LD A,0x05") && lines[0].contains("(7T)"),
        "{}",
        lines[0]
    );
    assert!(lines[1].contains("LD B,0x06"));
    assert!(lines[2].contains("LD (0x8000),A") && lines[2].contains("(13T)"));
    assert_eq!(m.cpu.regs.pc, 7);
}

#[test]
fn write_and_read_watchpoints() {
    let mut m = machine(PROG);
    let mut d = Debugger::new();
    run(&mut d, &mut m, "watch 0x8000");
    let r = d.run(&mut m, 1_000_000, u64::MAX);
    match r {
        StopReason::Watch(h) => {
            assert_eq!((h.addr, h.value, h.write), (0x8000, 5, true));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(m.cpu.regs.pc, 7); // tras completar el LD (nn),A
    // La escritura sí se realizó.
    assert_eq!(m.bus.memory.read(0x8000), 5);
    // Un watch de escritura no se dispara con la lectura siguiente...
    assert_eq!(d.run(&mut m, 10_000, u64::MAX), StopReason::TStates);
    // ...pero uno de lectura sí.
    let mut m = machine(PROG);
    let mut d = Debugger::new();
    run(&mut d, &mut m, "watch 0x8000 1 r");
    match d.run(&mut m, 1_000_000, u64::MAX) {
        StopReason::Watch(h) => assert_eq!((h.addr, h.write), (0x8000, false)),
        other => panic!("{other:?}"),
    }
    assert_eq!(m.cpu.regs.pc, 0x0A);
    assert_eq!(run(&mut d, &mut m, "watches"), "8000-8000 Read");
    run(&mut d, &mut m, "unwatch");
    assert_eq!(run(&mut d, &mut m, "watches"), "(sin watchpoints)");
    let _ = WatchKind::Access;
}

#[test]
fn watch_range_and_access_kind() {
    let mut m = machine(PROG);
    let mut d = Debugger::new();
    let out = run(&mut d, &mut m, "watch 0x7FF0 0x20 rw");
    assert!(out.contains("7FF0-800F") && out.contains("Access"), "{out}");
    assert!(matches!(
        d.run(&mut m, 1_000_000, u64::MAX),
        StopReason::Watch(_)
    ));
}

#[test]
fn regs_mem_dis_poke_set() {
    let mut m = machine(PROG);
    let mut d = Debugger::new();
    run(&mut d, &mut m, "step 2");
    let regs = run(&mut d, &mut m, "regs");
    assert!(
        regs.contains("PC=0004") && regs.contains("IM=0") && regs.contains("tstate=14"),
        "{regs}"
    );
    let mem = run(&mut d, &mut m, "mem 0 16");
    assert!(
        mem.starts_with("0000  3E 05 06 06 32 00 80 3A 00 80 18 FE"),
        "{mem}"
    );
    assert!(mem.contains("|>"), "{mem}");
    let dis = run(&mut d, &mut m, "dis 0 4");
    let l: Vec<&str> = dis.lines().collect();
    assert_eq!(l.len(), 4);
    assert!(l[0].contains("LD A,0x05") && l[2].contains("LD (0x8000),A"));
    assert!(l[2].starts_with(">"), "marca el PC: {}", l[2]);
    run(&mut d, &mut m, "poke $8000 $AB");
    assert_eq!(m.bus.memory.read(0x8000), 0xAB);
    run(&mut d, &mut m, "poke 0 0xFF"); // ROM: ignorado
    assert_eq!(m.bus.memory.read(0), 0x3E);
    run(&mut d, &mut m, "set hl 1234h");
    run(&mut d, &mut m, "set a 7");
    assert_eq!((m.cpu.regs.hl(), m.cpu.regs.a), (0x1234, 7));
    run(&mut d, &mut m, "set im 2");
    assert_eq!(m.cpu.im, 2);
}

#[test]
fn errors_do_not_panic_or_change_state() {
    let mut m = machine(PROG);
    let mut d = Debugger::new();
    for bad in [
        "frobnicate",
        "break",
        "break zz",
        "break 0x10000",
        "mem",
        "poke 0 300",
        "set zz 1",
        "set im 9",
        "step x",
        "profile foo",
    ] {
        let o = d.execute(&mut m, bad);
        assert!(o.text.starts_with("error:"), "{bad}: {}", o.text);
        assert!(!o.quit);
    }
    assert_eq!(m.tstate(), 0);
    assert!(d.execute(&mut m, "quit").quit);
    assert_eq!(d.execute(&mut m, "").text, "");
    assert!(d.execute(&mut m, "help").text.contains("watch"));
}

#[test]
fn cont_respects_frame_limit_and_frame_command() {
    let mut m = machine(&[0x18, 0xFE]); // JR $
    let mut d = Debugger::new();
    let out = run(&mut d, &mut m, "cont 2");
    assert!(out.starts_with("Límite alcanzado"), "{out}");
    assert!(m.tstate() >= 2 * 69_888 && m.tstate() < 2 * 69_888 + 100);
    run(&mut d, &mut m, "frame");
    assert_eq!(m.tstate() / 69_888, 3);
    assert_eq!(m.frame_count(), 3);
}

#[test]
fn profiler_attributes_tstates_to_hot_instructions() {
    // NOP x3 ; JR -5 (bucle de 3×4 + 12 = 24 T)
    let mut m = machine(&[0x00, 0x00, 0x00, 0x18, 0xFB]);
    let mut d = Debugger::new();
    assert!(run(&mut d, &mut m, "profile show").contains("no está activado"));
    run(&mut d, &mut m, "profile on");
    d.run(&mut m, 2400, u64::MAX);
    let rep = run(&mut d, &mut m, "profile show");
    assert!(rep.contains("JR 0x0000"), "{rep}");
    // El JR consume 12 de cada 24 T: ~50 %.
    let jr_line = rep.lines().find(|l| l.contains("JR 0x0000")).unwrap();
    assert!(
        jr_line.trim_start().starts_with("50.0%") || jr_line.trim_start().starts_with("49."),
        "{jr_line}"
    );
    assert!(rep.contains("0000-00FF"), "{rep}");
    run(&mut d, &mut m, "profile reset");
    assert!(run(&mut d, &mut m, "profile show").contains("0 instrucciones"));
    run(&mut d, &mut m, "profile off");
    assert!(!d.profiling());
}

#[test]
fn debugger_runs_do_not_alter_determinism() {
    let a = {
        let mut m = machine(PROG);
        m.run_tstates(50_000);
        (m.tstate(), m.cpu.regs)
    };
    let b = {
        let mut m = machine(PROG);
        let mut d = Debugger::new();
        d.start_profile();
        d.add_breakpoint(0xFFFF);
        d.run(&mut m, 50_000, u64::MAX);
        (m.tstate(), m.cpu.regs)
    };
    assert_eq!(a.1, b.1);
    assert!(b.0 >= 50_000);
}
