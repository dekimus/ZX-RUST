# zx48 — emulador fiel del ZX Spectrum 48K en Rust

Núcleo determinista (Z80 con temporización por ciclo, ULA con contención, floating bus, teclado,
beeper, cinta TAP/TZX, snapshots SNA/Z80/SCR) separado de la interfaz gráfica. Requiere una ROM
de 48K (`48.rom`, 16 KiB; SHA-1 estándar `5ea7c2b824672e914525d1d5c419d71b84a426a2`), que no se
incluye en el repositorio.

## Estado

Fases 1–7 completadas según [`AGENTS.md`](AGENTS.md):

* **Z80** completo (CB, ED, DD, FD, DDCB/FDCB, EI/DI con retardo de un instruction, IM 0/1/2) con
  coste en T-states por ciclo de máquina.
* **ULA**: frame de 69 888 T (312 líneas × 224 T), `/INT` de 32 T ≈ 50,08 Hz, vídeo 256×192 con
  border real por T-state, FLASH por reloj del emulador, contención `6,5,4,3,2,1,0,0` y floating
  bus.
* **Entrada/salida**: matriz de teclado 8×5 por el puerto `0xFE`, beeper con eventos por T-state,
  joystick Kempston opcional (conectado/desconectado en el puerto `0x1F`).
* **Cinta**: TAP y TZX con generación de señal EAR por T-states (cargador de ROM y cargadores
  turbo personalizados).
* **Snapshots**: SNA, Z80 (v1/v2/v3, 48K) y SCR, con rechazo explícito de los de 128K.
* **Herramientas**: desensamblador, depurador interactivo, trazas por categoría y CLI headless
  para CI.
* **Interfaz gráfica** (`eframe`/`egui`): menús, escalado entero 4:3 sin estiramiento, teclado
  del Spectrum, audio por ring buffer, cintas y snapshots con selectores, pantalla completa,
  capturas, ventana de depuración e icono propio.

Limitaciones conocidas (deliberadas):

* Solo 48K: sin banking de 128K, sin AY-3-8912, sin +2/+2A/+3.
* Sin carga rápida de cinta: `LOAD ""` ejecuta la rutina real de la ROM.
* La pausa entre bloques de un TAP vale 1 s fijo (`PAUSE_TSTATES`), no la duración del fichero.
* No se incluyen las flags intermedias de `INIR`/`OTIR` sin una referencia verificable
  (documentado en `src/cpu/z80.rs`).
* La fidelidad de raster está validada con tests propios, no aún contra capturas de hardware real.

## Uso rápido

```bash
# compilar (los binarios quedan en target/debug o target/release)
cargo build --release --bin zx48 --bin zx48-gui

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

# icono de la aplicación en el lanzador (Wayland: Plasma/GNOME)
scripts/install_desktop.sh
```

En la GUI: **F1** muestra los atajos, **F2** carga cintas, **F3** snapshots, **F5** pausa,
**F11** pantalla completa. Los atajos son solo F1–F12, Re/Av Pág y Esc: el resto del teclado es
el del Spectrum.

## Tests

```bash
cargo test                        # 207 pruebas: 174 unitarias + 33 de integración
cargo test --lib frontend         # solo la lógica del frontend, sin abrir ventana
cargo test --release --test zex -- --ignored   # zexdoc + zexall (2 pruebas ignoradas)
```

| Suite | Qué cubre |
|---|---|
| unitarias (`src/**`) | instrucciones y flags del Z80, timings, contención, vídeo, atributos, cinta, snapshots, teclado, audio y lógica del frontend |
| `tests/rom_boot.rs` | arranque de la ROM estándar, BASIC, cinta `demo.tap` |
| `tests/cli.rs` | banderas del CLI, volcados, hashes deterministas, errores |
| `tests/snapshot.rs` | SNA/Z80/SCR: ida y vuelta y rechazo de formatos no soportados |
| `tests/debug.rs` | depurador: breakpoints, watchpoints, desensamblado |
| `tests/zex.rs` | Z80 exerciser (zexdoc/zexall), ignoradas por tiempo |

Las que necesitan `48.rom` se saltan si el fichero no existe: un clone limpio compila y pasa.

## Documentación

* [`docs/UI.md`](docs/UI.md) — guía de uso: menús, atajos, teclado del Spectrum, cintas y
  snapshots, configuración, problemas frecuentes e icono.
* [`docs/FRONTEND.md`](docs/FRONTEND.md) — arquitectura del frontend (bucle, temporización,
  vídeo, entrada, audio y su separación del núcleo).
* [`AGENTS.md`](AGENTS.md) — especificación del proyecto: hardware, temporización, fases y
  criterios de aceptación.
* [`MEMORY.md`](MEMORY.md) — contexto persistente: constantes, hechos del hardware y reglas.

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
tests       integración: rom_boot, cli, snapshot, debug, zex (+ tests/data)
docs        UI.md (guía) y FRONTEND.md (arquitectura)
```

Features: `cli` (tracing para el binario `zx48`), `gui` (dependencias de la interfaz); ambas
por defecto. El núcleo no depende de egui, glutin ni cpal: `--headless` funciona sin GUI.

El reloj de pared solo se usa en el frontend y en `--bench`; el hardware virtual avanza por
T-states (69 888 por frame, ≈50.08 Hz). Dada la misma ROM, RAM y entrada, la ejecución produce
siempre los mismos T-states, framebuffer y hashes.
