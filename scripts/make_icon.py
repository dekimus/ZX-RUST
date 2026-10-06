#!/usr/bin/env python3
"""Regenera `assets/icon.png` a partir de `ico.jpeg`.

El JPEG original trae dibujado un damero gris en las esquinas (la típica "transparencia"
falsa que no lo es). Este script convierte ese damero en alfa real: recorta el logo a su
bounding box, redondea el borde y deja las esquinas transparentes, para que el icono se vea
limpio sobre cualquier fondo del escritorio.

Requisitos: Pillow y numpy (`python3 -c "import PIL, numpy"`), sin ImageMagick.

Uso (desde la raíz del repositorio o desde cualquier sitio):

    python3 scripts/make_icon.py
"""

from __future__ import annotations

import sys
from collections import deque
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "ico.jpeg"
DST = ROOT / "assets" / "icon.png"
SIZE = 256  # lado del PNG generado (múltiplo de 4, como exige egui::IconData)

# Rango del damero de fondo: dos grises neutros (≈106 y ≈158) con algo de ruido de JPEG.
CHROMA_MAX = 25
LUM_MIN, LUM_MAX = 92, 178


def checkerboard_alpha(image: Image.Image) -> Image.Image:
    """Alfa 0 en el damero conectado con el borde, 255 en el logo."""
    rgb = np.asarray(image.convert("RGB")).astype(np.int16)
    h, w, _ = rgb.shape
    chroma = rgb.max(axis=2) - rgb.min(axis=2)
    lum = rgb.mean(axis=2)
    neutral = (chroma <= CHROMA_MAX) & (lum >= LUM_MIN) & (lum <= LUM_MAX)

    # Flood fill desde los bordes: solo cuenta la región que toca el exterior, así que
    # los grises neutros que haya dentro del logo no se confunden con el damero.
    seen = np.zeros((h, w), dtype=bool)
    queue: deque[tuple[int, int]] = deque()
    for x in range(w):
        for y in (0, h - 1):
            if neutral[y, x]:
                seen[y, x] = True
                queue.append((y, x))
    for y in range(h):
        for x in (0, w - 1):
            if neutral[y, x] and not seen[y, x]:
                seen[y, x] = True
                queue.append((y, x))
    while queue:
        y, x = queue.popleft()
        for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            ny, nx = y + dy, x + dx
            if 0 <= ny < h and 0 <= nx < w and neutral[ny, nx] and not seen[ny, nx]:
                seen[ny, nx] = True
                queue.append((ny, nx))

    # Se dilata 1 px el logo para eliminar el residuo gris del borde antialias y se
    # suaviza para que el arco redondeado no quede con escalones.
    alpha = np.where(seen, 0, 255).astype(np.uint8)
    return (
        Image.fromarray(alpha, "L")
        .filter(ImageFilter.MaxFilter(3))
        .filter(ImageFilter.GaussianBlur(0.8))
    )


def main() -> int:
    if not SRC.exists():
        print(f"no existe {SRC}", file=sys.stderr)
        return 1

    image = Image.open(SRC).convert("RGB")
    alpha = checkerboard_alpha(image)

    arr = np.asarray(alpha)
    ys, xs = np.where(arr >= 128)
    box = (int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1)
    print(f"bbox: {box} ({box[2] - box[0]}x{box[3] - box[1]})")

    logo = Image.merge("RGBA", (*image.crop(box).split(), alpha.crop(box)))
    logo = logo.resize((SIZE, SIZE), Image.LANCZOS)

    DST.parent.mkdir(parents=True, exist_ok=True)
    logo.save(DST, optimize=True)
    print(f"escrito {DST.relative_to(ROOT)} ({SIZE}x{SIZE}, {DST.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
