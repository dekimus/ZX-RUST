#!/usr/bin/env bash
# Instala la entrada de escritorio y el icono de zx48 para el usuario actual.
#
# En sesiones Wayland (KDE Plasma, GNOME…) la ventana no puede enviar su icono: el
# escritorio lo toma del fichero .desktop cuyo nombre coincide con el `app_id` de la
# ventana (eframe lo fija en "zx48", el nombre pasado a run_native). Sin esta entrada,
# el icono solo se ve en X11 y en la ventana «Acerca de».
#
# Uso:  scripts/install_desktop.sh [ruta/al/zx48-gui]
# Deshacer:  borrar ~/.local/share/applications/zx48.desktop
#            y ~/.local/share/icons/hicolor/256x256/apps/zx48.png
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="zx48"
BIN="${1:-}"

if [[ -z "$BIN" ]]; then
    for candidate in "$ROOT/target/release/zx48-gui" "$ROOT/target/debug/zx48-gui"; do
        if [[ -x "$candidate" ]]; then
            BIN="$candidate"
            break
        fi
    done
fi
if [[ -z "$BIN" ]]; then
    BIN="$(command -v zx48-gui || true)"
fi
if [[ -z "$BIN" || ! -x "$BIN" ]]; then
    echo "no se encontró el binario zx48-gui; compila primero con: cargo build --bin zx48-gui" >&2
    exit 1
fi
BIN="$(readlink -f "$BIN")"

ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"
DESK_DIR="$HOME/.local/share/applications"
mkdir -p "$ICON_DIR" "$DESK_DIR"

# Icono (256x256, con alfa) con el nombre que declara el .desktop.
install -m 0644 "$ROOT/assets/icon.png" "$ICON_DIR/$APP_ID.png"

# Entrada de escritorio con la ruta absoluta al binario (para que funcione sin
# tener que instalar nada en PATH).
sed -e "s|^Exec=.*|Exec=$BIN %F|" \
    -e "s|^TryExec=.*|TryExec=$BIN|" \
    "$ROOT/packaging/zx48.desktop" > "$DESK_DIR/$APP_ID.desktop"
chmod 0644 "$DESK_DIR/$APP_ID.desktop"

if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate "$DESK_DIR/$APP_ID.desktop"
fi
if command -v update-desktop-database >/dev/null; then
    update-desktop-database "$DESK_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null; then
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi
# Plasma refresca su caché de servicios en cuanto aparece el fichero; por si acaso:
if command -v kbuildsycoca6 >/dev/null; then
    kbuildsycoca6 >/dev/null 2>&1 || true
elif command -v kbuildsycoca5 >/dev/null; then
    kbuildsycoca5 >/dev/null 2>&1 || true
fi

echo "icono    : $ICON_DIR/$APP_ID.png"
echo "entrada  : $DESK_DIR/$APP_ID.desktop"
echo "ejecuta  : $BIN"
echo
echo "Abre la app desde el lanzador (o reinicia la sesión) para ver el icono en Plasma."
