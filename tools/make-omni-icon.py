"""Generador del icono OMNI (minimalista B/N).

Concepto: la "O" de OMNI como anillo + nodo central.
- El anillo = omnipresencia / red (servidor + clientes).
- El punto central = la maquina que computa.
- Squircle negro #0B0D0F (fondo de la UI), marca blanca #F5F7F7.

Cada tamano del .ico se dibuja desde primitivas (nitido a 16 px),
no por reescalado. Variantes dark (app) y light (docs/fondos claros).
"""
from PIL import Image, ImageDraw

INK = (245, 247, 247, 255)      # blanco marca
PAPER_DARK = (11, 13, 15, 255)  # negro UI
PAPER_LIGHT = (245, 247, 247, 255)
INK_DARK = (11, 13, 15, 255)


def draw_mark(draw, s, paper, ink, small=False):
    # Fondo squircle.
    draw.rounded_rectangle([0, 0, s - 1, s - 1], radius=int(s * 0.225), fill=paper)
    # Anillo: mas grueso en tamanos chicos para que lea a 16 px.
    outer = 0.32 if small else 0.30
    width = max(2, int(round(s * (0.125 if small else 0.09))))
    r = int(round(s * outer))
    c = s / 2
    draw.ellipse([c - r, c - r, c + r, c + r], outline=ink, width=width)
    # Nodo central.
    dot = max(2, int(round(s * (0.095 if small else 0.075))))
    draw.ellipse([c - dot, c - dot, c + dot, c + dot], fill=ink)


def make_icon(s, paper, ink):
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    draw_mark(ImageDraw.Draw(img), s, paper, ink, small=(s <= 32))
    return img


def save_ico_multisize(images, path):
    """Ensambla un .ico real multi-tamano (entradas PNG, como hace Pillow
    por dentro). Cada tamano conserva su dibujo propio, no un reescalado."""
    import io
    import struct
    blobs = []
    for img in images:
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        blobs.append(buf.getvalue())
    n = len(images)
    header = struct.pack("<HHH", 0, 1, n)
    offset = 6 + 16 * n
    entries = b""
    for img, blob in zip(images, blobs):
        w, h = img.size
        entries += struct.pack(
            "<BBBBHHII",
            w if w < 256 else 0,
            h if h < 256 else 0,
            0,
            0,
            1,
            32,
            len(blob),
            offset,
        )
        offset += len(blob)
    with open(path, "wb") as f:
        f.write(header + entries + b"".join(blobs))


def make_master(s, paper, ink):
    # Master con margen (la marca respira, como en stores).
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    pad = int(s * 0.09)
    inner = make_icon(s - 2 * pad, paper, ink)
    img.alpha_composite(inner, (pad, pad))
    return img


if __name__ == "__main__":
    import os
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "omni")
    os.makedirs(out, exist_ok=True)

    # Masters 1024.
    make_master(1024, PAPER_DARK, INK).save(os.path.join(out, "omni.png"))
    make_master(1024, PAPER_LIGHT, INK_DARK).save(os.path.join(out, "omni-light.png"))

    # .ico multi-tamano: cada tamano dibujado desde primitivas.
    sizes = [16, 24, 32, 48, 64, 256]
    save_ico_multisize(
        [make_icon(s, PAPER_DARK, INK) for s in sizes],
        os.path.join(out, "omni.ico"),
    )
    # Vista previa de chequeo.
    make_icon(256, PAPER_DARK, INK).save(os.path.join(out, "preview-256.png"))
    print("OK:", sorted(os.listdir(out)))
