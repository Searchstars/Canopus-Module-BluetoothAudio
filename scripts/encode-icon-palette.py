#!/usr/bin/env python3
"""Pack the launcher icon (an LVGL v9 ARGB8888 BIN) as header + 256-colour palette + indices.

The module carries this ~15 KB form and expands it back to the ARGB8888 file the launcher reads,
instead of carrying the 54 KB image. Colours beyond 256 are merged into their nearest neighbour.
Index and alpha values 0x0c/0x1c are avoided: a word whose top byte is one of them reads like a
firmware address to the module verifier.

    encode-icon-palette.py watchfaces/bluetooth-audio/appicon_headphones.bin \
        crates/bluetooth-audio-device/assets/appicon_headphones.pal
"""
import struct
import sys
from collections import Counter

RESERVED = {0x0C, 0x1C}


def main(src, dst):
    data = open(src, "rb").read()
    header, pixels = data[:12], data[12:]
    magic, cf, _flags, w, h, stride = struct.unpack_from("<BBHHHH", header)
    assert (magic, cf, stride) == (0x19, 0x10, w * 4) and len(pixels) == w * h * 4, "not LVGL v9 ARGB8888"
    words = [struct.unpack_from("<I", pixels, i)[0] for i in range(0, len(pixels), 4)]

    def nudge(c):
        a = c >> 24
        return (c & 0xFFFFFF) | ((a + 1) << 24) if a in RESERVED else c

    words = [nudge(c) for c in words]
    counts = Counter(words)
    slots = 256 - len(RESERVED)

    def channels(c):
        a = c >> 24
        return [a] + [((c >> s) & 0xFF) * a // 255 for s in (16, 8, 0)]

    merged = {}
    colours = dict(counts)
    while len(colours) > slots:
        rare = min(colours, key=colours.get)
        rc = channels(rare)
        near = min((c for c in colours if c != rare),
                   key=lambda c: sum((x - y) ** 2 for x, y in zip(rc, channels(c))))
        colours[near] += colours.pop(rare)
        merged[rare] = near
        for k, v in merged.items():
            if v == rare:
                merged[k] = near

    def final(c):
        while c in merged:
            c = merged[c]
        return c

    order = sorted(colours, key=colours.get, reverse=True)
    palette, index = [0] * 256, {}
    free = (i for i in range(256) if i not in RESERVED)
    for colour in order:
        slot = next(free)
        palette[slot] = colour
        index[colour] = slot
    out = bytearray(header)
    out += b"".join(struct.pack("<I", c) for c in palette)
    out += bytes(index[final(c)] for c in words)
    open(dst, "wb").write(out)
    print(f"{len(counts)} colours -> {len(colours)}, {len(out)} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:3])
