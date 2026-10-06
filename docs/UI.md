# Interfaz de usuario — `zx48-gui`

Guía de uso del emulador gráfico. La arquitectura interna está en
[`FRONTEND.md`](FRONTEND.md).

## 1. Arranque

```bash
cargo run --release --features gui --bin zx48-gui -- 48.rom juego.tap
# o tras instalar:
zx48-gui [--allow-nonstandard-rom] [ROM] [archivo]
```

* **ROM** (opcional): fichero de 16 KiB. Si se omite se prueba la ruta guardada en la
  configuración y después `./48.rom`; si ninguna existe se abre un selector de ficheros.
  Una ROM indicada en la línea de comandos se usa aunque no exista, para que el error sea claro.
* **archivo** (opcional): `juego.tap` / `.tzx` (cinta) o `partida.sna` / `.z80` / `.scr`
  (snapshot) inicial. Un error aquí no impide arrancar: se muestra el motivo y la app sigue.
* `--help` imprime el uso.

La ventana se abre con la escala entera activa (1×, 2×…) y el tamaño calculado para que la
imagen quepa sin recortes. La ROM **no se distribuye**; en el repositorio no hay ROMs.

## 2. Distribución

```
┌────────────────────────────────────────────────────────────┐
│ Archivo · Emulación · Vídeo · Audio · Ayuda                │  menú
├────────────────────────────────────────────────────────────┤
│                                                            │
│      imagen del Spectrum (border real de la ULA)           │  área central
│                                                            │
├────────────────────────────────────────────────────────────┤
│ ● Funcionando │ Cinta: … │ Volumen: 80% │ Escala: 2× │ …   │  barra de estado
└────────────────────────────────────────────────────────────┘
```

### Menús

| Menú | Elementos |
|---|---|
| **Archivo** | Cargar cinta (F2) · Cargar snapshot (F3) · Guardar snapshot (F9) · Guardar captura (F12) · Seleccionar ROM… · Salir |
| **Emulación** | Pausa / reanudar (F5) · Paso de frame · Reiniciar (F4) · **Cinta** ▸ Play/Stop (F6), Rebobinar (F7), Expulsar · Joystick Kempston (flechas) |
| **Vídeo** | Escalado entero / Ajustar a la ventana · Border completo (352×296) / Border reducido 4:3 (320×240) / Sin border (256×192) · Filtro suave · Pantalla completa (F11) · Barra de estado · Información de depuración (F10) |
| **Audio** | Silenciar (F8) · Volumen (0–100 %) · Sonido de cinta |
| **Ayuda** | Atajos de teclado (F1) · Acerca de… |

Los cambios de Vídeo/Audio se aplican al instante y se guardan al salir.

### Barra de estado

* **● Funcionando / ● En pausa** — estado de la emulación.
* **Cinta** — `--` sin cinta; `archivo · bloque 2/3 · PLAY` con cinta cargada;
  `· fin` cuando se ha consumido el bloque final.
* **Volumen** — porcentaje actual (o `mute`).
* **Escala** — escala efectiva real de la imagen (p. ej. `2×`, o `1.67×` en modo ajuste).
* **Grupo derecho** — mensaje temporal («ROM cargada…», «Snapshot guardado…») mientras dura
  5 s; después `FPS · EMU Hz` en marcha, o `T-states: N` **en pausa** (el contador de FPS en
  pausa no significaría nada). En ventanas estrechas se ocultan primero Volumen y Escala para
  que los grupos nunca se pegen.

## 3. Atajos de teclado

Solo teclas de función y de navegación; **el resto del teclado es el del Spectrum**.

| Tecla | Acción | Tecla | Acción |
|---|---|---|---|
| F1 | Atajos de teclado | F7 | Rebobinar cinta |
| F2 | Cargar cinta | F8 | Silenciar |
| F3 | Cargar snapshot | F9 | Guardar snapshot (.sna) |
| F4 | Reiniciar | F10 | Ventana de depuración |
| F5 | Pausa / reanudar | F11 | Pantalla completa |
| F6 | Cinta play/stop | F12 | Guardar captura (.bmp) |
| Re Pág | Subir volumen | Av Pág | Bajar volumen |
| Esc | Salir de pantalla completa | | |

Anti-rebote: un mismo atajo repetido en menos de 300 ms se ignora (evita que un par
press/release duplicado alterne pausa→reanudar); el volumen sí puede mantenerse pulsado.

**Pausa** congela el Z80, el reloj (T-states), la imagen y el audio: no se ejecutan frames ni se
pide repintado hasta reanudar. **Reiniciar** usa el reset del núcleo (PC=0, IFF a cero, ROM
intacta, RAM sin tocar).

## 4. Teclado del Spectrum

Mapeo posicional: cada tecla del PC equivale a la del Spectrum de su misma posición.

| PC | Spectrum |
|---|---|
| Letras, dígitos, Enter, espacio | Misma tecla de la matriz |
| Mayús | **Caps Shift** |
| Ctrl o Alt | **Symbol Shift** |
| Retroceso | Caps Shift + 0 (borrar) |
| ← ↓ ↑ → | Cursores (Caps Shift + 5/6/7/8) o Kempston si está activo |
| Insert / Fin | Disparo Kempston (solo con Kempston) |
| `,` `.` `;` `-` `=` `/` `'` (sin Mayús) | Symbol Shift + N, M, O, J, L, V, 7 |

Notas prácticas de BASIC:

* En inicio de línea o tras `THEN`/`:` la ROM introduce la **palabra clave** de la tecla
  (p. ej. `P` → `PRINT`, `b` → `BORDER`); dentro de una línea o cadena se escribe la letra.
* Comillas rápidas: Ctrl (o Alt) + P → `"`. Así: `W` + Ctrl-P + Ctrl-P + Enter carga cinta
  (`LOAD ""`).
* Mayús + letras/dígitos = versátilas y símbolos rojos de la tecla; Mayús + 0 = borrar.
* Si la ventana pierde el foco **se sueltan todas las teclas** (nunca queda una pulsación
  «colgada»).

Kempston se activa con **Emulación ▸ Joystick Kempston (flechas)**: las flechas pasan a ser el
joystick (disparo: Insert o Fin) y el cursor se mueve con Caps Shift + 5/6/7/8.

La opción **conecta o desconecta la interfaz** en el puerto `0x1F`, no solo el mapeo del teclado.
Desconectada, ese puerto no puede reportar botones pulsados: un `.sna` guardado con la interfaz
puesta (el juego lo deja escrito en su RAM) se carga igualmente sin entradas fantasma.

## 5. Cintas, snapshots y capturas

* **Cargar cinta** (F2): selector nativo `.tap`/`.tzx`. La cinta entra por el core; el
  reproductor genera la señal EAR por T-states (compatible con cargadores ROM y turbo).
* Con **autoplay** activo (por defecto), cuando la ROM entra en su rutina de carga la cinta
  empieza a reproducirse sola: en la barra de estado pasa de `STOP` a `PLAY`.
* **F6** detiene/arranca la reproducción manualmente; **F7** rebobina al inicio; **Expulsar**
  retira la cinta del core.
* `LOAD ""` en BASIC es la ruta fiel (no se acelera la cinta).
* **Cargar snapshot** (F3): `.sna`, `.z80` (48K; los de 128K se rechazan con mensaje) y `.scr`
  (solo pantalla).
* **Guardar snapshot** (F9): siempre `.sna` con la RAM completa.
* **Guardar captura** (F12): `.bmp` de la salida actual (incluido el border visible).

Todos los errores (formato no soportado, fichero corrupto) aparecen en un diálogo de error, sin
crashes.

## 6. Configuración

Fichero `~/.config/zx48/settings.conf` (respeta `XDG_CONFIG_HOME`), `clave=valor`, se genera
solo y se escribe al salir:

```ini
volume=0.80        # volumen 0..1
muted=0
scale_mode=integer # integer | fit
border_mode=medium # full | medium | none
linear_filter=0    # filtro suave (por defecto: nearest-neighbor)
kempston=0         # 1 = flechas como joystick Kempston
tape_autoplay=1
tape_sound=1
show_status_bar=1
show_debug=0
window_scale=2
rom_path=48.rom
last_dir=...       # último directorio de los selectores
```

Claves desconocidas o inválidas se ignoran (valor por defecto), nunca rompen el arranque.

## 7. Rendimiento y determinismo

* El repintado se limita a ~50 FPS de forma controlada (`request_repaint_after`), sin `sleep`
  ni esperas activas; la emulación corre a sus 69 888 T-states/frame (~50.08 Hz).
* Si el sistema se retrasa, se recuperan como mucho 250 ms y 5 frames por repintado: la UI no
  se congela y la deuda no crece sin límite.
* No hay E/S de disco ni asignaciones en el bucle de repintado: los ficheros solo se tocan con
  acción del usuario (selector o guardar) y la configuración solo al salir.
* El core es determinista: pausar/reanudar no altera el estado; la misma ROM+entrada producen
  siempre la misma imagen y T-states.

## 8. Problemas frecuentes

| Síntoma | Causa / solución |
|---|---|
| «No se encontró la ROM de 16 KiB» | Pasa la ruta: `zx48-gui /ruta/48.rom`, o coloca `./48.rom`. |
| «La ROM no es la estándar…» | Usa otra ROM o `--allow-nonstandard-rom` (no se parchea ninguna ROM). |
| Sin sonido | La app sigue en silencio si el dispositivo de audio falla; revisa F8/`muted` y el menú Audio. |
| Las flechas no mueven el cursor | Con `kempston=1` son el joystick; desactívalo o usa Caps Shift + 5/6/7/8. |
| Una tecla «se queda» pulsada | Pulsa y suelta esa tecla de nuevo (o cambia de ventana y vuelve: al perder foco se suelta todo). |
| La cinta no arranca | F7 (rebobinar) y F6 (play), o escribe `LOAD ""` con la ROM; comprueba `tape_autoplay`. |

## 9. Icono

La ventana y la ventana **Ayuda ▸ Acerca de…** usan el logo del proyecto:

* **Origen:** `ico.jpeg` (raíz del repositorio), el asset original.
* **En el binario:** `assets/icon.png`, 256×256 con esquinas **realmente transparentes**
  (el JPEG trae un damero gris dibujado en las esquinas —la típica "transparencia" falsa—,
  que aquí se convierte en alfa). Va embebido con `include_bytes!`: no hay lectura de disco
  al arrancar ni en el bucle de repintado.
* **Módulo:** `src/frontend/icon.rs`, que lo decodifica con `eframe::icon_data`.
  Si el PNG no decodificara, la ventana se abre con el icono por defecto (sin `unwrap`).

Para regenerarlo cuando cambie `ico.jpeg` (requiere Pillow y numpy, sin ImageMagick):

```bash
python3 scripts/make_icon.py
```

El script recorta el logo a su caja delimitadora, sustituye el damero por transparencia,
suaviza el borde y escala a 256×256 (múltiplo de 4, como exige `egui::IconData`).

### Icono en Wayland (KDE Plasma, GNOME…)

Wayland no permite que la aplicación envíe su icono de ventana: el escritorio lo toma del
fichero `.desktop` cuyo nombre coincide con el `app_id` de la ventana (aquí, `zx48`).
Sin esa entrada, la barra de título y el dock muestran un icono genérico; en X11 sí se ve
el embebido. Para instalarla en el usuario actual:

```bash
scripts/install_desktop.sh          # usa target/release, después target/debug
scripts/install_desktop.sh /ruta/a/zx48-gui
```

Copia `assets/icon.png` a `~/.local/share/icons/hicolor/256x256/apps/zx48.png` y genera
`~/.local/share/applications/zx48.desktop` (a partir de `packaging/zx48.desktop`) con la
ruta absoluta al binario, refrescando además las cachés del escritorio. Para deshacer:

```bash
rm ~/.local/share/applications/zx48.desktop \
   ~/.local/share/icons/hicolor/256x256/apps/zx48.png
```
