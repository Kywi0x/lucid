#!/usr/bin/env python3
"""Génère src-tauri/icons/tray.png : template macOS 44x44 (22pt @2x),
noir + alpha sur fond transparent. Glyphe = constellation (nœud central +
satellites reliés), écho du graphe à bulles de Lucid. Stdlib only (zlib+struct).
Anti-aliasing par sur-échantillonnage 4x4."""
import zlib, struct, math, sys

W = H = 44
SS = 4  # sous-échantillons par axe

# Formes en espace 44x44
CENTER = (22.0, 22.0, 5.2)          # (cx, cy, r)
SATS = [(11.0, 11.0, 3.0), (33.0, 12.0, 3.0), (29.0, 32.0, 3.0)]
EDGES = [((22.0, 22.0), (s[0], s[1])) for s in SATS]
EDGE_HW = 1.1  # demi-épaisseur des liens

def in_circle(x, y, c):
    return (x - c[0]) ** 2 + (y - c[1]) ** 2 <= c[2] ** 2

def dist_seg(x, y, a, b):
    ax, ay = a; bx, by = b
    dx, dy = bx - ax, by - ay
    L2 = dx * dx + dy * dy
    if L2 == 0:
        return math.hypot(x - ax, y - ay)
    t = max(0.0, min(1.0, ((x - ax) * dx + (y - ay) * dy) / L2))
    return math.hypot(x - (ax + t * dx), y - (ay + t * dy))

def covered(x, y):
    if in_circle(x, y, CENTER):
        return True
    if any(in_circle(x, y, c) for c in SATS):
        return True
    return any(dist_seg(x, y, a, b) <= EDGE_HW for a, b in EDGES)

raw = bytearray()
for py in range(H):
    raw.append(0)  # filtre None
    for px in range(W):
        hits = 0
        for sy in range(SS):
            for sx in range(SS):
                x = px + (sx + 0.5) / SS
                y = py + (sy + 0.5) / SS
                if covered(x, y):
                    hits += 1
        alpha = round(255 * hits / (SS * SS))
        raw += bytes((0, 0, 0, alpha))  # noir, alpha variable (template)

def chunk(tag, data):
    return (struct.pack(">I", len(data)) + tag + data +
            struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

png = b"\x89PNG\r\n\x1a\n"
png += chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
png += chunk(b"IEND", b"")

with open(sys.argv[1], "wb") as f:
    f.write(png)
print(f"écrit {sys.argv[1]} ({len(png)} octets, {W}x{H} RGBA)")
