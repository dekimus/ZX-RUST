# zx48 — emulador fiel del ZX Spectrum 48K en Rust

Núcleo determinista (Z80 con temporización por ciclo, ULA con contención, floating bus, teclado,
beeper, cinta TAP/TZX, snapshots SNA/Z80/SCR) separado de la interfaz gráfica. Requiere una ROM
de 48K (`48.rom`, 16 KiB; SHA-1 estándar `5ea7c2b824672e914525d1d5c419d71b84a426a2`), que no se
incluye en el repositorio.

## Uso rápido

```bash
cargo test                                   # unitarios + integración (usa 48.rom si existe)
cargo test --release --test zex -- --ignored # zexdoc/zexall (≈40 s cada uno)

# GUI (eframe/egui): pantalla, menús, teclado del Spectrum, beeper y cinta
cargo run --release --bin zx48-gui -- 48.rom
cargo run --release --bin zx48-gui -- 48.rom juego.tap     # cinta o snapshot inicial
cargo run --release --bin zx48-gui -- --help

# CLI headless (CI, depuración, trazas)
cargo run --release --bin zx48 -- --rom 48.rom --frames 100 --hash --headless
cargo run --release --bin zx48 -- --rom 48.rom --frames 500 --bench
cargo run --release --bin zx48 -- --rom 48.rom --tstates 200 --trace-cpu 2>trace.log
cargo run --release --bin zx48 -- --rom 48.rom --debug      # depurador interactivo (help)
cargo run --release --bin zx48 -- --rom 48.rom --frames 300 --profile
```

En la GUI: **F1** muestra los atajos, **F2** carga cintas, **F5** pausa, **F11** pantalla
completa. La documentación completa de la interfaz está en [`docs/UI.md`](docs/UI.md) y la
arquitectura del frontend en [`docs/FRONTEND.md`](docs/FRONTEND.md).

## Estructura

```
src/cpu       Z80 (z80.rs), desensamblador, registros, flags
src/machine   Spectrum48, bus (reloj maestro y contención), memoria
src/ula       reloj de frame, vídeo con border por T-state, contención, floating bus, imagen
src/input     matriz de teclado, Kempston (opcional)
src/audio     beeper (eventos) y remuestreador determinista
src/tape      TAP, TZX y generación de la señal EAR
src/snapshot  SNA, Z80 (v1/v2/v3, 48K), SCR
src/debug     breakpoints, watchpoints, perfil
src/frontend  lógica pura del frontend (viewport, ajustes, atajos, teclado, audio, icono) [feature `gui`]
src/bin/gui.rs  binario `zx48-gui` (eframe/egui + glutin + cpal + rfd)             [feature `gui`]
assets      icono 256×256 con esquinas transparentes (a partir de ico.jpeg)
packaging   zx48.desktop (entrada de escritorio; la instala scripts/install_desktop.sh)
scripts     make_icon.py (regenera el icono) e install_desktop.sh (icono en Wayland)
```

Features: `cli` (tracing para el binario `zx48`), `gui` (dependencias de la interfaz); ambas
por defecto. El núcleo no depende de egui, glutin ni cpal: `--headless` funciona sin GUI.

El reloj de pared solo se usa en el frontend y en `--bench`; el hardware virtual avanza por
T-states (69 888 por frame, ≈50.08 Hz).
