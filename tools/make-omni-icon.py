"""Icono OMNI v2: esfera de grafito + orbita (la O de OMNI).

Blanco y negro como la UI, pero con volumen: esfera con sombreado radial
suave + anillo orbital inclinado que la cruza. Todo en grises, sin color.

Anti-pixelado: se dibuja a 2048 (supersampling) con numpy para el degradado
y se baja a 1024 con LANCZOS; las entradas del .ico salen del master con
LANCZOS (practica estandar: bordes suaves, no escalones).
"""
import os

import numpy as np
from PIL import Image, ImageDraw, ImageOps

INK = (245, 247, 247)
PAPER = (11, 13, 15)

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'assets', 'omni')
SS = 2048  # lienzo de trabajo (supersampling)


def rounded_bg(s, radius_frac=0.225):
    img = Image.new('RGB', (s, s), (0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, s - 1, s - 1], radius=int(s * radius_frac), fill=PAPER)
    return img.convert('L')


def sphere_layer(s):
    """Esfera gris con luz arriba-izquierda y terminador suave abajo-derecha."""
    y, x = np.mgrid[0:s, 0:s].astype(np.float64)
    c = s / 2.0
    r = s * 0.285
    dx = (x - c) / r
    dy = (y - c) / r
    dist = np.sqrt(dx * dx + dy * dy)
    # luz: gaussiana desplazada hacia arriba-izquierda
    hx, hy = -0.38, -0.42
    hl = np.exp(-(((dx - hx) ** 2 + (dy - hy) ** 2) / (2 * 0.42 ** 2)))
    # cuerpo: cae hacia el borde (terminador)
    body = np.clip(1.0 - dist ** 2.6, 0.0, 1.0) ** 0.8
    val = 16.0 + 165.0 * body + 110.0 * hl * body
    val = np.clip(val, 0, 255)
    # alfa: borde de esfera nitido pero antialiaseado (transicion de ~2px a 2048)
    edge = np.clip((1.0 - dist) * (s / 2.0) / 2.5 + 0.5, 0.0, 1.0)
    rgb = np.stack([val, val, val], axis=-1).astype(np.uint8)
    alpha = (np.clip(edge, 0, 1) * 255).astype(np.uint8)
    img = Image.fromarray(rgb, 'RGB')
    img.putalpha(Image.fromarray(alpha, 'L'))
    return img


def orbit_layer(s):
    """Anillo orbital inclinado: mitad superior tenue (detras de la esfera),
    mitad inferior brillante (delante). Sin angulos magicos: dos elipses
    completas y se borra la mitad que corresponde."""
    c = s / 2.0
    box = [c - s * 0.40, c - s * 0.155, c + s * 0.40, c + s * 0.155]
    back = Image.new('RGBA', (s, s), (0, 0, 0, 0))
    ImageDraw.Draw(back).ellipse(box, outline=(150, 154, 160, 255), width=int(s * 0.015))
    front = Image.new('RGBA', (s, s), (0, 0, 0, 0))
    df = ImageDraw.Draw(front)
    df.ellipse(box, outline=(245, 247, 247, 255), width=int(s * 0.020))
    df.rectangle([0, 0, s, int(c)], fill=(0, 0, 0, 0))  # arriba la aporta `back`
    orb = Image.alpha_composite(back, front)
    return orb.rotate(-18, resample=Image.BICUBIC, center=(c, c))


def compose(dark=True):
    bg = rounded_bg(SS)
    base = Image.merge('RGB', (bg, bg, bg))
    sph = sphere_layer(SS)
    base.paste(sph, (0, 0), sph)
    orb = orbit_layer(SS)
    base = Image.alpha_composite(base.convert('RGBA'), orb)
    # margen de respiro y bajada al master
    pad = int(SS * 0.07)
    full = Image.new('RGBA', (SS, SS), (0, 0, 0, 0))
    inner = base.crop((pad, pad, SS - pad, SS - pad))
    full.alpha_composite(inner, (pad, pad))
    master = full.resize((1024, 1024), Image.LANCZOS)
    if not dark:
        r, g, b, a = master.split()
        inv = ImageOps.invert(Image.merge('RGB', (r, g, b)))
        master = Image.merge('RGBA', (*inv.split(), a))
    return master.convert('RGB')


def save_ico_multisize(images, path):
    import io
    import struct
    blobs = []
    for img in images:
        buf = io.BytesIO()
        img.save(buf, format='PNG')
        blobs.append(buf.getvalue())
    n = len(images)
    header = struct.pack('<HHH', 0, 1, n)
    offset = 6 + 16 * n
    entries = b''
    for img, blob in zip(images, blobs):
        w, h = img.size
        entries += struct.pack('<BBBBHHII', w if w < 256 else 0, h if h < 256 else 0,
                               0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
    with open(path, 'wb') as f:
        f.write(header + entries + b''.join(blobs))


if __name__ == '__main__':
    os.makedirs(OUT, exist_ok=True)
    dark = compose(dark=True)
    dark.save(os.path.join(OUT, 'omni.png'))
    compose(dark=False).save(os.path.join(OUT, 'omni-light.png'))
    sizes = [16, 24, 32, 48, 64, 256]
    save_ico_multisize([dark.resize((s, s), Image.LANCZOS) for s in sizes],
                       os.path.join(OUT, 'omni.ico'))
    dark.resize((256, 256), Image.LANCZOS).save(os.path.join(OUT, 'preview-256.png'))
    print('OK v2:', sorted(os.listdir(OUT)))
