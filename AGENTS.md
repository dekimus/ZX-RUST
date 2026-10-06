# AGENTS.md — ZX Spectrum 48K Emulator in Rust

## 0. Objetivo

Construir un emulador fiel del **Sinclair ZX Spectrum 48K original**, orientado inicialmente al hardware PAL estándar de 48K y escrito en Rust.

La prioridad es, en este orden:

1. Corrección funcional del Z80 y del mapa de memoria.
2. Temporización correcta por T-state.
3. ULA y vídeo 256×192 con atributos de 8×8.
4. Interrupción de 50.08 Hz y contención de memoria/I/O.
5. Teclado Sinclair y puerto `0xFE`.
6. Beeper/EAR/MIC.
7. Carga de cinta (`.tap`) y reproducción temporal del EAR.
8. Snapshots `.sna` y `.z80`.
9. Compatibilidad con software real y suites de pruebas.
10. Separación estricta entre núcleo emulado y frontend.

No implementar primero una "máquina virtual rápida" que ignore la temporización y luego intentar corregirla: muchos programas del Spectrum dependen de la relación exacta CPU/ULA.

---

## 1. Alcance del hardware

### 1.1 Máquina objetivo

Emular exclusivamente el **ZX Spectrum 16K/48K original**, en configuración de 48K RAM.

Especificaciones relevantes:

- CPU: Zilog Z80A.
- Frecuencia nominal CPU: 3.500.000 Hz.
- ROM: 16 KiB, `0x0000..0x3FFF`.
- RAM: 48 KiB, `0x4000..0xFFFF`.
- Espacio de direcciones CPU: 64 KiB.
- ULA: vídeo, teclado, cinta, beeper y generación de reloj/interrupción.
- Vídeo: 256×192 píxeles.
- Área de atributos: 32×24 celdas.
- 8 colores base; cada atributo dispone de INK/PAPER, BRIGHT y FLASH.
- Sonido interno: beeper de 1 bit controlado mediante `OUT` al puerto ULA.
- Entrada de cinta: EAR.
- Salida de cinta: MIC.
- Teclado: matriz de 8 filas × 5 columnas.
- Conector de expansión externo.
- Salida RF/TV del hardware original; el frontend moderno puede generar RGB/RGBA/texture.

### 1.2 Qué NO pertenece al objetivo inicial

No introducir en el núcleo 48K:

- 128K banking.
- AY-3-8912.
- +2/+2A/+3.
- Interface 1/2.
- Microdrive.
- Disciple/Plus D.
- Multiface.
- Timex TC2048/TC2068.
- Sam Coupé.
- clones con ULA distinta.

Diseñar interfaces que permitan añadirlos posteriormente, pero no contaminar el modelo 48K.

---

## 2. Fuentes técnicas de referencia

Usar estas fuentes como documentación primaria/secundaria:

- World of Spectrum — 48K ZX Spectrum Technical Information:
  https://www.worldofspectrum.net/faq/reference/48kreference.htm
- World of Spectrum — Zilog Z80 Technical Reference:
  https://worldofspectrum.org/faq/reference/z80reference.htm
- World of Spectrum — Z80 snapshot format:
  https://www.worldofspectrum.net/faq/reference/z80format.htm
- World of Spectrum — SNA/TAP y formatos:
  https://www.worldofspectrum.net/zx-modules/fileformats/snaformat.html
  https://worldofspectrum.net/zx-modules/fileformats/tapformat.html
- World of Spectrum — formatos de emulador:
  https://www.worldofspectrum.net/faq/reference/formats.htm
- ZX Spectrum Service Manual:
  https://spectrumforeveryone.com/wp-content/uploads/2017/08/ZX-Spectrum-Service-Manual.pdf
- Sinclair Wiki — imágenes ROM:
  https://sinclair.wiki.zxnet.co.uk/wiki/ROM_images
- Spectrum for Everyone — esquemas PCB:
  https://www.spectrumforeveryone.com/technical/zx-spectrum-pcb-schematics-layout/
- Sinclair Wiki — memoria contended:
  https://sinclair.wiki.zxnet.co.uk/wiki/Contended_memory

Las fuentes anteriores son material de consulta. No asumir que una implementación existente es correcta solo porque pasa juegos básicos.

---

## 3. Modelo de memoria

Mapa exacto:

```text
0x0000 ┌──────────────────────────────┐
       │ ROM 16 KiB                   │
0x3FFF ├──────────────────────────────┤
       │ RAM 16 KiB                  │
       │ incluye display file        │
0x7FFF ├──────────────────────────────┤
       │ RAM 16 KiB                  │
0xBFFF ├──────────────────────────────┤
       │ RAM 16 KiB                  │
0xFFFF └──────────────────────────────┘
```

En Rust, el bus debe distinguir:

```rust
0x0000..=0x3fff => rom[address]
0x4000..=0xffff => ram[(address - 0x4000)]
```

Escribir en ROM no debe modificarla.

### 3.1 RAM visible

Las 48 KiB son una región lineal desde `0x4000` hasta `0xFFFF`.

La ULA comparte con la CPU la región de vídeo en `0x4000..0x5AFF`, por lo que la RAM no debe modelarse simplemente como memoria sin temporización.

---

## 4. ROM

El emulador debe recibir la ROM como recurso externo.

No incluir una ROM Sinclair redistribuible dentro del repositorio salvo que exista una licencia explícita que lo permita.

ROM estándar de 16 KiB:

- Tamaño: `16384` bytes.
- SHA-1 de referencia:
  `5ea7c2b824672e914525d1d5c419d71b84a426a2`
- MD5 de referencia:
  `4c42a2f075212361c3117015b107ff68`

Validar:

1. existe el fichero;
2. tiene exactamente 16384 bytes;
3. opcionalmente coincide con SHA-1 conocido;
4. si no coincide, permitir cargarlo con `--allow-nonstandard-rom` o equivalente.

Nunca "parchear" automáticamente una ROM desconocida.

---

## 5. Z80

El núcleo Z80 debe ser independiente de la máquina.

### 5.1 Registros

Implementar:

- `AF`, `BC`, `DE`, `HL`
- `AF'`, `BC'`, `DE'`, `HL'`
- `IX`, `IY`
- `SP`, `PC`
- `I`, `R`
- `IFF1`, `IFF2`
- `IM 0`, `IM 1`, `IM 2`

Soportar instrucciones:

- principales;
- CB;
- ED;
- DD;
- FD;
- DD CB / FD CB;
- instrucciones no documentadas relevantes para software real.

No simplificar flags de instrucciones rotas o indocumentadas sin pruebas.

### 5.2 EI/DI

`EI` no habilita una interrupción arbitrariamente antes de tiempo.

Respetar la semántica Z80: una interrupción aceptable después de `EI` se reconoce después de la instrucción siguiente.

`DI` limpia IFF1/IFF2 según la semántica del Z80.

### 5.3 Interrupciones

Para el Spectrum 48K:

- ULA genera una interrupción aproximadamente cada frame.
- frame = `69888` T-states.
- frecuencia nominal = `3_500_000 / 69888 ≈ 50.08 Hz`.
- `/INT` se mantiene bajo durante 32 T-states.
- la CPU debe muestrear la interrupción en los puntos definidos por el Z80.

Implementar IM0/IM1/IM2 correctamente aunque la ROM use normalmente IM1.

En IM1, el comportamiento esperado del Spectrum es entrar en `0x0038`.

No "llamar a 0x0038" como una operación abstracta: modelar el comportamiento de aceptación de interrupción del Z80, incluyendo consumo de T-states.

---

## 6. Temporización

### 6.1 Reloj maestro

El emulador debe tener un contador global de T-states:

```rust
u64 tstate;
```

No usar `Instant` ni el reloj del sistema como fuente de verdad de la máquina.

El frontend sincroniza el emulador con tiempo real; el hardware virtual no.

### 6.2 Frame

Para el 48K PAL:

```text
CPU clock             3.500 MHz
T-states/frame        69,888
scanlines/frame       312
T-states/scanline     224
frame rate             ~50.08 Hz
```

Distribución vertical:

```text
64 líneas antes del área visible
192 líneas de imagen
56 líneas posteriores
----------------------
312 líneas
```

Cada scanline tiene:

```text
128 T   zona de 256 píxeles de imagen
24 T    borde derecho
48 T    retrazo horizontal
24 T    borde izquierdo
----------------------
224 T
```

La descripción temporal de la ULA es más importante que la simple geometría del framebuffer.

### 6.3 Inicio de la pantalla

Desde la interrupción de frame, pasan:

```text
64 * 224 = 14336 T-states
```

hasta el comienzo del primer byte visible de la pantalla.

El primer byte de pantalla está en `0x4000`.

---

## 7. ULA

La ULA debe ser un componente con estado propio:

```rust
struct Ula {
    tstate_in_frame: u32,
    border: u8,
    mic: bool,
    ear_output: bool,
    ear_input: bool,
    flash_phase: bool,
    ...
}
```

La ULA es responsable de:

- generación de vídeo;
- generación de `/INT`;
- acceso periódico a display RAM;
- contención CPU;
- lectura de teclado a través de `0xFE`;
- EAR;
- salida MIC;
- salida al altavoz/beeper.

No duplicar estas responsabilidades en el frontend.

---

## 8. Display RAM

El display ocupa 6912 bytes:

```text
0x4000..0x57FF  6144 bytes de bitmap
0x5800..0x5AFF   768 bytes de atributos
```

### 8.1 Bitmap

256×192 píxeles = 6144 bytes.

Cada byte contiene 8 píxeles.

El layout NO es lineal por scanline.

Para una coordenada `(x, y)`:

```text
x_byte = x >> 3

address =
    0x4000
    | ((y & 0xC0) << 5)
    | ((y & 0x07) << 8)
    | ((y & 0x38) << 2)
    | x_byte
```

Verificar esta fórmula con tests de direcciones:

```text
(x=0,   y=0)   -> 0x4000
(x=0,   y=1)   -> 0x4100
(x=0,   y=7)   -> 0x4700
(x=0,   y=8)   -> 0x4020
(x=0,   y=64)  -> 0x4800
(x=0,   y=128) -> 0x5000
(x=248, y=191) -> 0x57FF
```

### 8.2 Atributos

```text
attribute = ram[0x5800 + (y >> 3) * 32 + (x >> 3)]
```

Bits:

```text
7     FLASH
6     BRIGHT
5..3  PAPER
2..0  INK
```

Colores base:

```text
0 black
1 blue
2 red
3 magenta
4 green
5 cyan
6 yellow
7 white
```

BRIGHT selecciona la variante brillante.

FLASH intercambia INK y PAPER periódicamente.

El framebuffer debe ser capaz de renderizar el estado actual de FLASH; no convertir los atributos en píxeles permanentes.

---

## 9. Puerto ULA `0xFE`

El Spectrum tiene decodificación parcial: el hardware ULA responde a puertos pares; `0xFE` es la dirección canónica que debe utilizar el software.

### 9.1 OUT

En `OUT (0xFE)`:

```text
bit 0..2  BORDER color
bit 3      MIC
bit 4      EAR / speaker
bit 5..7   unused
```

El cambio de border ocurre con precisión temporal y puede producir raster effects.

El emulador no debe actualizar solamente un `border` por frame. Debe asociar los cambios al T-state en el que finaliza el `OUT`.

### 9.2 IN

En lectura de `0xFE`:

- A15..A8 seleccionan las filas de teclado.
- D0..D4 devuelven las cinco teclas de la fila.
- lógica activa en bajo: `0 = pulsada`.
- D6 = EAR input.
- D5/D7 dependen del bus/floating-bus y no deben tratarse como siempre cero sin justificarlo.

Filas:

```text
0xFEFE -> SHIFT, Z, X, C, V
0xFDFE -> A, S, D, F, G
0xFBFE -> Q, W, E, R, T
0xF7FE -> 1, 2, 3, 4, 5
0xEFFE -> 0, 9, 8, 7, 6
0xDFFE -> P, O, I, U, Y
0xBFFE -> ENTER, L, K, J, H
0x7FFE -> SPACE, SYMBOL SHIFT, M, N, B
```

Si varias filas están seleccionadas simultáneamente, las entradas se combinan como AND eléctrico.

---

## 10. Teclado

Modelo:

```rust
struct Keyboard {
    // 8 rows × 5 active-low columns
    rows: [u8; 8],
}
```

Estado interno recomendado:

- `1` = no pulsada;
- `0` = pulsada.

No implementar primero teclas "ASCII"; el hardware no conoce caracteres, conoce posiciones en una matriz.

El frontend debe mapear teclado físico a las teclas Spectrum.

Permitir combinaciones como:

- SHIFT + letra;
- SYMBOL SHIFT + tecla;
- SHIFT + 0 = backspace;
- etc.

La interpretación semántica de combinaciones pertenece a la ROM/BASIC, no al teclado emulado.

---

## 11. Contención de memoria

Esto es obligatorio para una emulación de calidad.

En 16K/48K:

```text
0x4000..0x7FFF = contended RAM
0x8000..0xFFFF = uncontended RAM
```

La ULA roba ciclos de CPU durante el periodo de vídeo.

Patrón clásico de espera durante el acceso contendido:

```text
6, 5, 4, 3, 2, 1, 0, 0
```

El patrón se sincroniza con el inicio del área de display y se repite por scanline.

Una referencia habitual expresa la primera ventana contended como:

```text
14335 -> +6
14336 -> +5
14337 -> +4
14338 -> +3
14339 -> +2
14340 -> +1
```

y continúa según la secuencia anterior.

IMPORTANTE:

- El retraso se aplica al acceso contendido, no "a la instrucción completa" de forma arbitraria.
- La memoria contended debe ser consciente del T-state.
- I/O contendido necesita su propia lógica.
- No calcularlo únicamente al final de cada instrucción.

Diseñar la API de bus para poder decir:

```rust
fn memory_read(&mut self, addr: u16) -> (u8, u32);
fn memory_write(&mut self, addr: u16, value: u8) -> u32;
fn io_read(&mut self, port: u16) -> (u8, u32);
fn io_write(&mut self, port: u16, value: u8) -> u32;
```

o equivalente, de forma que el CPU y el bus compartan el reloj.

---

## 12. Floating bus

El Spectrum tiene comportamiento de "floating bus" en lecturas de puertos no conectados y en ciertas situaciones de vídeo.

No devolver `0xFF` indiscriminadamente.

El valor leído puede corresponder al último byte que la ULA está colocando en el bus de vídeo.

Esto importa para:

- efectos raster;
- protecciones;
- demos;
- detección de modelo;
- software que explota el hardware.

Implementar floating bus después de tener correcta la temporización de vídeo, porque depende de qué byte está leyendo la ULA en ese T-state.

---

## 13. Beeper y audio

El sonido interno es un altavoz de un bit.

`OUT 0xFE` bit 4 cambia el nivel del beeper.

El audio no es un PSG: no hay frecuencia interna.

El frontend debe convertir transiciones del bit 4 en muestras o eventos de audio.

No generar una onda fija por cada `OUT`. El estado real es una señal digital cuya frecuencia depende del programa.

Mantener:

```rust
struct AudioEvent {
    tstate: u64,
    level: bool,
}
```

o un buffer equivalente.

El EAR y MIC son señales relacionadas físicamente; para emulación de cinta debe modelarse explícitamente la señal de entrada EAR.

---

## 14. Vídeo

Separar:

1. memoria de vídeo;
2. reloj ULA;
3. decodificador de píxeles;
4. framebuffer;
5. presentación del frontend.

Resolución lógica:

```text
256 × 192
```

El border forma parte de la salida visual y no debe ignorarse si se quiere compatibilidad con efectos de raster.

Un frontend puede presentar:

- 256×192;
- 352×288 aproximado con border;
- 640×480 escalado;
- textura RGBA.

Pero el core debe trabajar con coordenadas y tiempos del hardware, no con píxeles del frontend.

---

## 15. Flash

El atributo FLASH alterna INK/PAPER.

La tasa de flash debe derivarse del sistema temporal del Spectrum, no de un timer del sistema operativo.

Mantener una fase de flash ligada al frame count.

La implementación exacta debe validarse contra la ROM y software real.

---

## 16. Cinta

### 16.1 `.tap`

Implementar primero TAP estándar.

Un TAP contiene una secuencia de bloques con:

```text
2 bytes little-endian = longitud
N bytes = bloque
```

Los bloques estándar ROM tienen:

- flag;
- datos;
- checksum XOR.

Cabecera estándar de ROM:

```text
flag = 0x00
type:
  0 PROGRAM
  1 number array
  2 character array
  3 CODE
filename = 10 bytes
length = 2 bytes
param1 = 2 bytes
param2 = 2 bytes
checksum = XOR
```

Un `SCREEN$` es un bloque CODE de:

```text
start = 16384
length = 6912
```

### 16.2 Cargador ROM

Debe existir una ruta de compatibilidad que permita que la ROM ejecute su rutina real de carga.

No saltarse siempre la ROM.

El modo "fast tape load" puede existir como aceleración explícita, pero debe estar separado del modo fiel.

### 16.3 Señal temporal

Para software con turbo/custom loaders:

```text
TAP -> pulsos -> EAR
```

El reproductor temporal debe generar cambios de nivel de EAR en T-states.

No asumir que todos los programas usan el cargador ROM estándar.

Para TZX, diseñar un parser independiente de la emulación de señal y soportar progresivamente bloques que permitan generar pulsos.

---

## 17. Snapshots

### 17.1 SNA 48K

Un SNA 48K contiene:

- registros;
- IFF2;
- R;
- IM;
- border;
- 49152 bytes RAM.

La RAM representa:

```text
0x4000..0xFFFF
```

En el formato SNA 48K, el PC se obtiene de la pila: el loader debe reproducir la semántica documentada y preparar la CPU para continuar con `RETN`.

No confundir SNA con Z80.

### 17.2 Z80

Soportar:

- versión 1, 48K;
- versión 2;
- versión 3 cuando describe hardware 48K.

En Z80 v1:

- header inicial de 30 bytes;
- PC está en header;
- RAM puede estar comprimida con `ED ED count value`;
- fin de snapshot comprimido con `00 ED ED 00`.

En v2/v3:

- PC inicial = 0 en el header base;
- existe header extendido;
- después aparecen bloques de memoria de 16 KiB;
- en modo 48K interesan las páginas correspondientes a RAM 0x4000–0xFFFF.

El parser debe rechazar bloques malformados en vez de leer fuera de rango.

---

## 18. Arquitectura Rust recomendada

Separar crates/módulos:

```text
src/
  cpu/
    mod.rs
    z80.rs
    registers.rs
    flags.rs
    opcode.rs
    timing.rs

  machine/
    mod.rs
    spectrum48.rs
    bus.rs
    memory.rs

  ula/
    mod.rs
    timing.rs
    video.rs
    contention.rs
    floating_bus.rs

  input/
    keyboard.rs

  audio/
    beeper.rs

  tape/
    mod.rs
    tap.rs
    tzx.rs
    signal.rs

  snapshot/
    mod.rs
    sna.rs
    z80.rs

  rom/
    loader.rs

  frontend/
    // SDL2, winit/wgpu, egui, etc.; fuera del core
```

No acoplar el core a SDL2/wgpu.

### 18.1 Estructura de máquina

Una posible estructura:

```rust
pub struct Spectrum48 {
    pub cpu: Z80,
    pub ula: Ula,
    pub memory: Memory48,
    pub keyboard: Keyboard,
    pub tape: TapeDevice,
    pub beeper: Beeper,
    pub tstate: u64,
}
```

Sin embargo, si el Z80 necesita mutar el bus durante una instrucción, evitar préstamos Rust imposibles mediante un diseño de `Bus`/`MachineBus`.

Una arquitectura tipo:

```rust
cpu.step(&mut bus);
```

es preferible a exponer toda la máquina al CPU.

---

## 19. Contrato del bus

El CPU debe conocer solo:

```rust
trait Bus {
    fn mem_read(&mut self, addr: u16) -> u8;
    fn mem_write(&mut self, addr: u16, value: u8);

    fn io_read(&mut self, port: u16) -> u8;
    fn io_write(&mut self, port: u16, value: u8);

    fn interrupt_line(&self) -> bool;

    fn tick(&mut self, tstates: u32);
}
```

El bus es responsable de avanzar la máquina.

Una alternativa más precisa es hacer que cada operación devuelva el coste de T-states y que un scheduler central avance la ULA.

No elegir una API que obligue a introducir hacks posteriormente para la contención.

---

## 20. Estrategia de temporización CPU

Hay dos niveles posibles:

### Nivel A — instruction stepping

Cada instrucción devuelve sus T-states.

Útil para:

- arrancar rápido;
- ejecutar ROM;
- validar registros.

### Nivel B — micro-op/T-state accurate

Cada M-cycle y acceso relevante avanza el reloj.

Necesario para:

- contention;
- raster effects;
- floating bus;
- border timing;
- custom loaders;
- demos.

Objetivo final: Nivel B.

Se puede empezar en A si existe una ruta clara de migración.

---

## 21. Estado y determinismo

El emulador debe ser determinista.

Dado:

```text
ROM
RAM inicial
estado CPU
estado ULA
estado teclado
estado cinta
```

la ejecución debe producir siempre la misma secuencia de:

- T-states;
- framebuffer;
- audio;
- interrupciones;
- lecturas de bus.

No depender del reloj wall-clock.

---

## 22. Inicialización/reset

Reset debe establecer el estado necesario del hardware.

Como mínimo:

- PC = 0;
- SP en estado definido por el modelo/reset;
- IFF = disabled;
- IM = estado definido por Z80/reset;
- memoria no debe inventarse como ceros si el objetivo es fidelidad histórica.

Si el frontend quiere "power-on randomization", debe ser una opción separada.

Para tests deterministas se puede usar RAM inicializada a cero.

---

## 23. Testing

### 23.1 CPU

Crear tests para:

- todas las instrucciones documentadas;
- flags;
- instrucciones CB/ED/DD/FD;
- prefijos;
- DDCB/FDCB;
- HALT;
- EI/DI;
- IM0/1/2;
- interrupciones;
- NMI si el núcleo Z80 lo soporta.

Usar tests externos conocidos cuando sea posible.

### 23.2 Memoria

Tests:

```text
ROM write ignored
0x4000 first RAM byte
0xFFFF last RAM byte
screen boundaries
attribute boundaries
```

### 23.3 Vídeo

Verificar la fórmula de dirección de bitmap.

Generar una pantalla donde:

- cada byte de bitmap tenga patrón único;
- cada atributo tenga color único;
- se puedan localizar errores de orden.

### 23.4 ULA

Test de:

- frame = 69888 T;
- scanline = 224 T;
- interrupt timing;
- border change timing;
- contention;
- floating bus;
- EAR;
- keyboard matrix.

### 23.5 Cinta

TAP:

- parseo;
- checksum;
- header;
- data;
- SCREEN$;
- bloques vacíos;
- bloque corrupto.

### 23.6 Snapshots

Tests de round-trip:

```text
machine state -> SNA -> load -> equivalent state
machine state -> Z80 -> load -> equivalent state
```

Comparar RAM completa y registros.

---

## 24. ROM smoke tests

Una vez cargada la ROM estándar:

1. reset;
2. ejecutar suficientes T-states para pasar el arranque;
3. comprobar que aparece el prompt BASIC;
4. escribir/ejecutar pequeñas rutinas;
5. probar `PRINT`, `POKE`, `PEEK`, `BEEP`, `LOAD`.

La ROM estándar de 48K debe ser una prueba de integración del CPU + bus + ULA.

No sustituirla por un BASIC propio para validar la emulación.

---

## 25. Tests de raster imprescindibles

Implementar programas de prueba que:

1. cambien border mediante `OUT 254,n`;
2. cambien border varias veces dentro de un frame;
3. escriban en `0x4000..0x7FFF` durante display;
4. lean puertos flotantes;
5. ejecuten bucles sincronizados con la interrupción;
6. generen música en el beeper;
7. usen el cargador de cinta ROM;
8. usen un loader turbo basado en EAR.

Los efectos deben compararse a capturas/trace de un emulador de referencia o hardware real.

---

## 26. Rendimiento

No optimizar antes de tener tests de fidelidad.

Después:

- usar arrays contiguos;
- evitar allocations por T-state;
- evitar `dyn Trait` en loops calientes si el perfil demuestra coste;
- separar frontend y core;
- permitir ejecutar varias instrucciones por batch cuando sea seguro;
- mantener una ruta exacta para tests.

La optimización no debe cambiar los resultados observables.

---

## 27. API pública sugerida

```rust
pub struct Spectrum48 {
    // ...
}

impl Spectrum48 {
    pub fn new(rom: [u8; 16 * 1024]) -> Self;

    pub fn reset(&mut self);

    pub fn step(&mut self);

    pub fn run_tstates(&mut self, count: u64);

    pub fn run_frame(&mut self);

    pub fn framebuffer(&self) -> &[u8];

    pub fn key_down(&mut self, key: SpectrumKey);

    pub fn key_up(&mut self, key: SpectrumKey);

    pub fn insert_tape(&mut self, tape: Tape);

    pub fn save_sna(&self) -> Vec<u8>;
    pub fn load_sna(&mut self, data: &[u8]) -> Result<(), SnapshotError>;

    pub fn load_z80(&mut self, data: &[u8]) -> Result<(), SnapshotError>;
}
```

No exponer directamente los detalles de la ULA al frontend salvo mediante una API de diagnóstico.

---

## 28. CLI de desarrollo

Proponer:

```text
zx48
  --rom 48.rom
  --tape game.tap
  --snapshot game.z80
  --debug
  --trace-cpu
  --trace-ula
  --dump-screen screen.scr
  --frames 100
  --tstates 1000000
```

El modo headless es obligatorio para CI.

Ejemplo:

```bash
zx48 --rom 48.rom --snapshot test.z80 --frames 10 --headless
```

---

## 29. Logging/trace

Crear categorías:

```text
CPU
BUS
ULA
VIDEO
IO
TAPE
AUDIO
SNAPSHOT
```

Nunca hacer `println!` en el loop normal.

Usar `tracing` o equivalente.

Para depuración:

```text
tstate=14336 pc=.... event=FRAME_VISIBLE_START
tstate=14339 pc=.... event=BORDER_CHANGE value=2
```

---

## 30. Errores

Usar tipos de error explícitos:

```rust
enum EmulatorError {
    RomSize,
    RomChecksum,
    InvalidSnapshot,
    UnsupportedSnapshot,
    InvalidTape,
    CpuInvariant,
}
```

No usar `unwrap()` para input del usuario.

`unwrap()` puede ser aceptable en invariantes internas demostradas por tests.

---

## 31. Compatibilidad de snapshots

No asumir que todos los `.z80` son 48K.

Validar hardware type.

Si el snapshot es 128K:

```text
return UnsupportedHardware
```

en la implementación inicial.

Nunca cargar silenciosamente solo una parte de un snapshot 128K.

---

## 32. Formato `.scr`

Soportar opcionalmente:

```text
6912 bytes = dump de 0x4000..0x5AFF
```

Es extremadamente útil para tests del renderer.

---

## 33. Orden de implementación

### Fase 1
- proyecto Rust;
- CPU Z80;
- memoria;
- ROM loader;
- máquina 48K básica;
- tests CPU.

### Fase 2
- ULA;
- frame clock;
- interrupt;
- vídeo;
- border.

### Fase 3
- keyboard;
- port FE;
- beeper.

### Fase 4
- contention;
- I/O contention;
- floating bus;
- raster accuracy.

### Fase 5
- TAP;
- EAR;
- ROM loader real;
- TZX progresivo.

### Fase 6
- SNA;
- Z80;
- SCR.

### Fase 7
- frontend;
- debugger;
- tracing;
- profiling.

---

## 34. Criterios de aceptación

El proyecto no se considera un emulador 48K completo hasta que:

- la ROM arranca;
- BASIC funciona;
- el vídeo coincide;
- el teclado funciona;
- el beeper produce sonido;
- `.tap` estándar carga;
- `.sna` carga;
- `.z80` 48K carga;
- la interrupción tiene la frecuencia/posición correcta;
- la contención está implementada;
- los raster effects básicos funcionan;
- el modo headless pasa CI;
- el core no depende del frontend.

---

## 35. Errores conceptuales que evitar

### NO hacer

```rust
sleep(20ms);
interrupt();
```

La máquina virtual debe avanzar por T-states.

### NO hacer

```rust
if addr < 0x8000 { cpu_cycles += 1; }
```

La contención depende del instante del acceso.

### NO hacer

```rust
framebuffer[y * 256 + x] = ...
```

sin implementar primero el layout real de memoria.

### NO hacer

```rust
port_fe_read() -> 0xff
```

para todas las lecturas.

### NO hacer

cargar un `.tap` simplemente copiando bytes a RAM si el objetivo es fidelidad.

### NO hacer

meter 128K/AY/+3 en la primera versión.

---

## 36. Notas sobre variantes de hardware

El ZX Spectrum 48K tuvo revisiones de placa y diferentes ULAs.

El objetivo inicial debe definirse como:

```text
48K PAL-compatible timing profile
```

y no como una emulación analógica de un número concreto de issue de placa.

Si más adelante aparecen diferencias entre Issue 1/2/3/4/5/6:

- documentarlas;
- modelarlas como `HardwareProfile`;
- no introducir `if issue == ...` repartidos por todo el core.

---

## 37. Fuentes y hechos clave que el agente debe recordar

- 16 KiB ROM + 48 KiB RAM.
- RAM total visible: `0x4000..0xFFFF`.
- Display file: `0x4000..0x5AFF`, 6912 bytes.
- Bitmap: 6144 bytes.
- Attributes: 768 bytes.
- CPU: Z80A, 3.5 MHz.
- 312 scanlines/frame.
- 224 T-states/scanline.
- 69888 T-states/frame.
- ~50.08 Hz.
- 64 scanlines antes de imagen.
- 192 scanlines visibles.
- 56 scanlines después.
- puerto ULA: `0xFE` / puertos pares parcialmente decodificados.
- teclado: 8×5.
- border: bits 0..2.
- MIC: bit 3.
- EAR/beeper: bit 4.
- EAR input: bit 6 en lectura.
- contended RAM: `0x4000..0x7FFF`.
- no existe banking en 48K.
- no existe AY.
- snapshot SNA 48K = estado + 49152 bytes RAM.
- TAP estándar = bloques prefijados por longitud de 16 bits little-endian.
- Z80 v1 puede comprimir RAM con `ED ED count value`.

---

## 38. Regla de oro

Si hay que elegir entre:

```text
"funciona con juegos sencillos"
```

y

```text
"modela correctamente el hardware"
```

elegir el segundo.

El Spectrum 48K parece simple porque tiene pocos componentes, pero la combinación Z80 + ULA + memoria contended + vídeo sincronizado hace que la temporización sea parte de la funcionalidad.
