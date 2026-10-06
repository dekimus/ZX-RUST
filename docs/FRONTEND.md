# Frontend gráfico (eframe/egui)

Este documento describe la arquitectura del frontend gráfico **`zx48-gui`**: cómo se separa del
núcleo emulado, cómo se temporiza, cómo entra el teclado y cómo se reproduce el audio. La guía de
uso (menús, atajos, barra de estado) está en [`UI.md`](UI.md).

## 1. Principios

1. **El frontend no emula nada.** No hay CPU, ULA, memoria, contención ni temporización en
   `src/frontend/`. Toda la simulación vive en el núcleo (`src/cpu`, `src/machine`, `src/ula`,
   …) y el frontend solo la *alimenta* y la *muestra*.
2. **El núcleo sigue siendo headless.** `cargo run --bin zx48 -- --frames 100 --headless`
   no toca egui, glutin ni cpal; el núcleo no depende de ninguna de estas crates.
3. **Reloj del sistema solo para sincronizar, nunca como verdad.** El reloj del Spectrum avanza
   por T-states (69 888 por frame); `std::time::Instant` se usa únicamente para decidir *cuántos*
   frames toca ejecutar en cada repintado y para medir FPS.
4. **Lógica pura testeable sin ventana.** `viewport`, `settings`, `actions`, `keymap`, `icon`
   y `audio` (salvo la apertura del dispositivo) son funciones puras con tests unitarios;
   `app.rs` es la única pieza que habla con egui/eframe.

## 2. Módulos

```
src/frontend/
  mod.rs        reexportes
  app.rs        App de eframe: bucle, menús, ventanas, diálogos, barra de estado  (~1100 líneas)
  viewport.rs   mapeo región→UV, escalado entero/ajuste, tamaño de ventana        (tests)
  settings.rs   Settings + persistencia clave=valor en XDG config                 (tests)
  actions.rs    Action, tabla de atajos, anti-rebote de 300 ms                     (tests)
  keymap.rs     teclado físico → matriz del Spectrum / Kempston, diff por frame     (tests)
  audio.rs      productor de muestras (ring buffer), mezcla beeper+cinta, volumen   (tests)
  icon.rs       icono embebido (`assets/icon.png`) para ventana y «Acerca de»      (tests)
src/bin/gui.rs  main: argumentos, resolución de ROM, fichero inicial, run_native
```

Dependencias (feature `gui`): `eframe 0.36` (egui + glutin), `rfd` (selectores nativos),
`cpal` (audio), `ringbuf` (buffer de audio).

El `app_id` de la ventana es el nombre pasado a `run_native` (`zx48`): es la clave con la que un
escritorio Wayland asocia `packaging/zx48.desktop` para mostrar el icono (en X11 el icono va
embebido con `ViewportBuilder::with_icon`). Detalle en [`UI.md`](UI.md) §9.

## 3. Orden del bucle (`App::ui`)

Cada repintado ejecuta, en este orden:

1. **Entrada**: lee `ui.input` (teclas pulsadas, modificadores, foco), calcula el conjunto de
   teclas del Spectrum con `keymap::held_from` y despacha las **diferencias** al core
   (`key_down`/`key_up`, `joy_down`/`joy_up`). Sin foco (o con un campo de texto de egui
   enfocado) el conjunto es vacío ⇒ **se sueltan todas las teclas**.
2. **Atajos**: `actions::shortcut` + `actions::debounced` (ventana de 300 ms) → cola `pending`.
3. **Ajustes**: aplica `Settings` modificados por los menús al core (volumen, mute, …).
4. **`tick_emulation`**: ejecuta los frames que toquen (ver §4) y encola audio.
5. **`update_texture`**: si `machine.frame_count()` cambió, recopia el framebuffer a la textura
   egui con `TextureOptions::NEAREST` (o `LINEAR` si «Filtro suave»). No hay asignaciones por
   T-state ni E/S de disco en el repintado.
6. Estadísticas (FPS medidos), menús, barra de estado, pintado de la pantalla (§5) y ventanas.
7. **Repintado**: `request_repaint_after(wait + predicted_dt)` con
   `wait = FRAME_TIME - acc` (eframe descuenta `predicted_dt`). En pausa no se pide repintado.

## 4. Temporización de la ejecución

```rust
pub const FRAME_TIME: Duration = Duration::from_micros(19_968); // 69888 T @ 3.5 MHz
pub const MAX_FRAMES_PER_UPDATE: u32 = 5;
const MAX_CATCHUP: Duration = Duration::from_millis(250);
```

* El acumulador `acc` guarda la deuda respecto al tiempo real; `frames_to_run(acc, max)` devuelve
  cuántos frames ejecutar y el resto.
* La deuda se limita a `MAX_CATCHUP` (si el sistema se bloquea, se descarta tiempo en lugar de
  acumular frames) y a `MAX_FRAMES_PER_UPDATE` por repintado, para que la UI nunca se congele.
* No hay `sleep(20ms)` ni esperas activas: la sincronización la hace `request_repaint_after`.
* Medido en vivo: 50 FPS de repintado y ~50.0 Hz de emulación.

## 5. Vídeo

* El framebuffer del core incluye el border real de la ULA (no es un dibujo del frontend).
* `viewport::place` calcula la región visible, el escalado y la posición en píxeles físicos:
  * **Escalado entero** (por defecto): múltiplos exactos 1×/2×/3×… con barras si no cabe;
  * **Ajustar a la ventana**: escala fraccionaria máxima conservando la proporción.
* La proporción 4:3 la da la región *Border reducido*: 320×240 (256×192 de pantalla + 32 px a
  los lados y 24 arriba/abajo). Opciones: `Full` (352×296), `Medium` (320×240), `None` (256×192).
* El recorte a textura UV se hace con `Region::uv()`: solo se sube al shader la parte visible.
* Sin estiramiento y sin tearing: el blit es un quad texturizado con nearest-neighbor por
  defecto; el FLASH parpadea con el reloj del emulador (contador de frames), no con timers del SO.
* El tamaño inicial de ventana = región × escala + `UI_CHROME` (48 px de menú + barra de estado).

## 6. Entrada (teclado y joystick)

Modelo **por estado**, no por eventos:

* Cada repintado se calcula `Held { spectrum, joy }` a partir de las teclas que egui reporta
  pulsadas y de los modificadores; `keymap::diff(prev, next)` produce solo las transiciones, con
  orden estable (determinista).
* La repetición automática del sistema operativo no genera pulsaciones extra (mismo estado ⇒
  diff vacío). La pérdida de foco vacía el conjunto y **suelta todas las teclas**.
* La matriz va al core (`SpectrumKey` → fila/columna activa-baja); el frontend no conoce bits.

Equivalencias (posiciónales + comodidad):

| Tecla PC | Spectrum |
|---|---|
| Letras, dígitos, Enter, espacio | Misma posición en la matriz |
| Mayús | Caps Shift |
| Ctrl o Alt | Symbol Shift |
| Retroceso | Caps Shift + 0 (borrar) |
| Flechas | Caps Shift + 5/6/7/8 (cursores) o joystick Kempston si está conectado |
| Insert / Fin | Disparo del Kempston (solo con Kempston activo) |
| `, . ; - = / '` (sin Mayús) | Symbol Shift + N, M, O, J, L, V, 7 |

Las teclas F1..F12 y Re Pág/Av Pág son **exclusivas de la aplicación** (nunca llegan al Spectrum);
Ctrl/Alt/Combinación no desencadenan atajos para no pisar Symbol Shift (ver `actions.rs`).

## 7. Audio

* El core expone eventos del beeper por T-state; el frontend los remuestrea a la frecuencia del
  dispositivo (`audio::ring`, remuestreo determinista) y los empuja a un ring buffer.
* `AudioProducer::fill` (hilo de cpal) solo lee del ring buffer; si va corto inserta silencio —
  **nunca bloquea** al hilo de emulación ni hace E/S.
* Hay dos fuentes mezcladas: beeper y sonido de cinta (`audio::mix`), con `tape_sound` separado.
* Volumen/mute viven en `AudioControl` (atómicos compartidos con el productor); el slider del
  menú Audio y Re Pág/Av Pág (pasos del 10 %) lo actualizan sin reiniciar el dispositivo.
* Si el dispositivo de audio no está disponible, la app sigue funcionando en silencio
  (`AudioOutput::open` devuelve `Err` y se muestra el motivo, sin `unwrap`).

## 8. Configuración

`Settings` (volumen, mute, escalado, border, filtro, Kempston, autoplay de cinta, barra de
estado, ventana, ROM, directorio último) se serializa como `clave=valor` en

```text
$XDG_CONFIG_HOME/zx48/settings.conf   (~/.config/zx48/settings.conf)
```

* Se carga al arrancar y **solo se escribe en `on_exit`** (nunca en el bucle de repintado).
* Parseo tolerante: claves desconocidas o valores inválidos caen al valor por defecto; los tests
  cubren ida y vuelta completa.
* Sin `unwrap` en rutas de fichero: los errores de lectura/escritura se ignoran o se muestran.

## 9. Manejo de ficheros

| Acción | Vía |
|---|---|
| Cinta `.tap`/`.tzx` | selector `rfd` → `machine.insert_tape(...)` (core) |
| Snapshot `.sna`/`.z80`/`.scr` | selector `rfd` → `machine.load_file(...)` (core) |
| Guardar snapshot | `machine.save_sna()` → escritura del selector |
| Captura `.bmp` | `ula::image::to_bmp(framebuffer)` → escritura del selector |
| ROM | selector `rfd` → `rom::loader::load_rom` → `Spectrum48::new` (máquina reiniciada) |

Los selectores se abren solo con acción explícita del usuario (nunca automáticamente en el
arrancado). La ROM inicial de la línea de comandos se usa aunque no exista, para que el error
sea claro.

## 10. Errores

* Sin `unwrap`/`expect` en rutas normales: los `Result` del core (ROM, snapshots, cinta) se
  traducen en un diálogo modal `Error` con botón «Cerrar», o en un mensaje de la barra de estado.
* El fichero inicial de la CLI que falla no impide arrancar: se muestra el motivo y la máquina
  queda usable.

## 11. Tests

`cargo test --lib frontend` cubre (sin abrir ventana):

* `viewport`: regiones, UV, escalado entero/ajuste, encaje 4:3, `window_size_for`.
* `settings`: ida y vuelta, valores por defecto, fichero corrupto, ruta XDG.
* `actions`: atajos únicos y etiquetados, teclas del Spectrum nunca disparan acciones,
  anti-rebote (incluido reloj no monotónico), volumen repetible.
* `keymap`: biyección de las 38 teclas, modificadores, combos de puntuación, cursores/Kempston,
  diff estable, foco perdido ⇒ todo suelto, sin repeticiones.
* `audio`: ring buffer, mezcla, ganancia, remuestreo.
* `icon`: el PNG embebido decodifica, mide 256×256 (múltiplo de 4) y sus esquinas tienen alfa 0.
* `app`: `frames_to_run`, `FRAME_TIME == 19 968 µs`, constantes de pacing.

La validación visual en vivo se hace contra un Xvfb con capturas de la ventana (pantalla de
arranque, borde con `BORDER 2`, teclado BASIC, pausa, barra de estado).
