import math
from pathlib import Path
from PIL import Image, ImageDraw

def create_localmind_assets(output_dir: Path = None):
    if output_dir is None:
        output_dir = Path(__file__).resolve().parent
    output_dir.mkdir(parents=True, exist_ok=True)
    
    # 1. Generate High-Res 1024x1024 Icon
    size = 1024
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    
    cx, cy = size // 2, size // 2
    
    # Outer ambient glow
    glow_layers = [
        (460, (99, 102, 241, 15)),
        (440, (6, 182, 212, 25)),
        (420, (14, 165, 233, 40)),
        (400, (99, 102, 241, 60)),
    ]
    for r, col in glow_layers:
        draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=col)
        
    # Squircle / Rounded shield container
    pad = 96
    box = [pad, pad, size - pad, size - pad]
    radius = 210
    
    # Border gradient / outer rim
    draw.rounded_rectangle(box, radius=radius, fill=(10, 14, 26, 255), outline=(56, 189, 248, 220), width=10)
    
    # Inner subtle rim
    draw.rounded_rectangle([pad + 12, pad + 12, size - pad - 12, size - pad - 12], radius=radius - 8, outline=(99, 102, 241, 120), width=3)
    
    # Digital matrix rings / neural arcs
    arc_draw = ImageDraw.Draw(img)
    for r, width, col in [(280, 2, (30, 41, 59, 140)), (210, 2, (51, 65, 85, 120)), (140, 2, (71, 85, 105, 100))]:
        arc_draw.ellipse([cx - r, cy - r, cx + r, cy + r], outline=col, width=width)
        
    # Neural / Mind Graph structure
    nodes = [
        # Center core
        (cx, cy, 32, (255, 255, 255, 255), (56, 189, 248, 255)),
        
        # Left cluster
        (cx - 100, cy - 60, 18, (129, 140, 248, 255), (99, 102, 241, 220)),
        (cx - 110, cy + 70, 18, (56, 189, 248, 255), (6, 182, 212, 220)),
        (cx - 170, cy - 10, 16, (168, 85, 247, 255), (147, 51, 234, 220)),
        (cx - 70, cy - 160, 16, (56, 189, 248, 255), (14, 165, 233, 220)),
        (cx - 160, cy - 130, 14, (129, 140, 248, 255), (99, 102, 241, 220)),
        (cx - 200, cy - 70, 12, (56, 189, 248, 255), (6, 182, 212, 200)),
        (cx - 210, cy + 40, 12, (168, 85, 247, 255), (147, 51, 234, 200)),
        (cx - 150, cy + 140, 14, (56, 189, 248, 255), (6, 182, 212, 220)),
        (cx - 70, cy + 180, 14, (129, 140, 248, 255), (99, 102, 241, 220)),
        (cx, cy - 190, 18, (255, 255, 255, 255), (56, 189, 248, 255)),
        (cx, cy + 190, 18, (255, 255, 255, 255), (129, 140, 248, 255)),

        # Right cluster
        (cx + 100, cy - 60, 18, (129, 140, 248, 255), (99, 102, 241, 220)),
        (cx + 110, cy + 70, 18, (56, 189, 248, 255), (6, 182, 212, 220)),
        (cx + 170, cy - 10, 16, (168, 85, 247, 255), (147, 51, 234, 220)),
        (cx + 70, cy - 160, 16, (56, 189, 248, 255), (14, 165, 233, 220)),
        (cx + 160, cy - 130, 14, (129, 140, 248, 255), (99, 102, 241, 220)),
        (cx + 200, cy - 70, 12, (56, 189, 248, 255), (6, 182, 212, 200)),
        (cx + 210, cy + 40, 12, (168, 85, 247, 255), (147, 51, 234, 200)),
        (cx + 150, cy + 140, 14, (56, 189, 248, 255), (6, 182, 212, 220)),
        (cx + 70, cy + 180, 14, (129, 140, 248, 255), (99, 102, 241, 220)),
    ]
    
    connections = [
        (0, 1), (0, 2), (0, 10), (0, 11), (0, 12), (0, 13),
        (1, 3), (1, 4), (3, 2), (2, 8), (2, 9), (4, 10), (4, 5), (5, 6), (6, 7), (7, 8), (8, 9), (9, 11),
        (3, 6), (3, 7),
        (12, 14), (12, 15), (14, 13), (13, 19), (13, 20), (15, 10), (15, 16), (16, 17), (17, 18), (18, 19), (19, 20), (20, 11),
        (14, 17), (14, 18),
        (4, 15), (9, 20), (1, 12), (2, 13)
    ]
    
    for i, j in connections:
        x1, y1 = nodes[i][0], nodes[i][1]
        x2, y2 = nodes[j][0], nodes[j][1]
        draw.line([x1, y1, x2, y2], fill=(56, 189, 248, 50), width=9)
        draw.line([x1, y1, x2, y2], fill=(99, 102, 241, 90), width=5)
        draw.line([x1, y1, x2, y2], fill=(224, 242, 254, 200), width=2)
        
    for x, y, r, inner_col, glow_col in nodes:
        draw.ellipse([x - r - 10, y - r - 10, x + r + 10, y + r + 10], fill=(glow_col[0], glow_col[1], glow_col[2], 60))
        draw.ellipse([x - r - 4, y - r - 4, x + r + 4, y + r + 4], fill=(glow_col[0], glow_col[1], glow_col[2], 160))
        draw.ellipse([x - r, y - r, x + r, y + r], fill=inner_col)
        
    dw = 18
    draw.polygon([(cx, cy - dw - 16), (cx + dw, cy), (cx, cy + dw + 16), (cx - dw, cy)], fill=(255, 255, 255, 255))
    
    png_path = output_dir / "localmind.png"
    img.save(png_path, format="PNG")
    
    sizes = [256, 128, 64, 48, 32, 16]
    ico_imgs = [img.resize((s, s), Image.Resampling.LANCZOS) for s in sizes]
    ico_path = output_dir / "localmind.ico"
    ico_imgs[0].save(ico_path, format="ICO", sizes=[(s, s) for s in sizes], append_images=ico_imgs[1:])
    print(f"Generated assets at: {output_dir}")

if __name__ == "__main__":
    create_localmind_assets()
