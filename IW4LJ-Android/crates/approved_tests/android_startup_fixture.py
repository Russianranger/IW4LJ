#!/usr/bin/env python3
"""Generate a tiny synthetic PC IW4 installation for Android startup QA."""

import argparse
import json
import struct
import zipfile
import zlib
from pathlib import Path

INLINE = 0xffffffff
MENU_FILES = (
    "frontend", "connection_error", "map_error", "classes", "barracks",
    "settings", "controller", "game_folders", "community_servers",
)
# Five rows per letter; each octal digit is a three-pixel row.
ALPHABET = dict(zip("ABCDEFGHIJKLMNOPQRSTUVWXYZ", (
    "25755", "65656", "34443", "65556", "74647", "74644", "34553",
    "55755", "72227", "11152", "55655", "44447", "57755", "57775",
    "25552", "65644", "25573", "65655", "34216", "72222", "55557",
    "55552", "55775", "55255", "55222", "71247",
)))


def words(*values):
    return struct.pack(f"<{len(values)}I", *values)


def field(record, offset, value):
    struct.pack_into("<I", record, offset, value)


def cstr(s):
    return s.encode("ascii") + b"\0"


def source_menus():
    """Supply the base items referenced by this revision's frontend patches."""
    directory = Path(__file__).resolve().parents[1] / "ui/menus"
    menus = {"main": [], "main_text": []}
    defined = set()
    for filename in MENU_FILES:
        for definition in json.loads((directory / f"{filename}.json").read_text()):
            base = definition.get("base")
            if base and base not in defined:
                items = menus.setdefault(base, [])
                for patch in definition.get("item_overrides", []) + definition.get("item_copies", []):
                    if patch.get("optional"):
                        continue
                    if "index" in patch:
                        while len(items) <= patch["index"]:
                            items.append({"name": f"item{len(items)}"})
                    else:
                        selector = {k: v for k, v in patch.items() if k not in ("fields", "optional")}
                        if not any(all(item.get(k) == v for k, v in selector.items()) for item in items):
                            items.append({"name": f"item{len(items)}", **selector})
            defined.add(definition["name"])
    return menus


def menu_record(name, items):
    # 32-bit menuDef_t and itemDef_t records, with strings and children inline.
    menu = bytearray(400)
    field(menu, 0, INLINE)
    field(menu, 172, len(items))
    field(menu, 280, INLINE if items else 0)
    body = menu + cstr(name) + words(INLINE) * len(items)
    for item in items:
        record = bytearray(380)
        field(record, 184, item.get("item_type", 0))
        strings = bytearray()
        for key, offset in (("name", 0), ("text_key", 228), ("dvar", 272)):
            if item.get(key):
                field(record, offset, INLINE)
                strings.extend(cstr(item[key]))
        body.extend(record + strings)
    return body


def bitmap_font():
    # A 64x64 alpha atlas. Glyph metrics map both cases to these generated letters.
    size = 64
    pixels = bytearray(size * size)
    glyphs = bytearray()
    for index, codepoint in enumerate(range(32, 128)):
        x, y = (index % 16) * 4, (index // 16) * 8
        rows = ALPHABET.get(chr(codepoint).upper(), "00000" if codepoint == 32 else "72102")
        for row, mask in enumerate(rows):
            for column in range(3):
                if int(mask) & (4 >> column):
                    pixels[(y + row) * size + x + column] = 255
        glyphs.extend(struct.pack("<HbbBBBB4f", codepoint, 0, -5, 4, 3, 5, 0,
                                  x / size, y / size, (x + 3) / size, (y + 5) / size))

    # The inline font material references an IWD-backed 2D image. One state-bits
    # row enables colour writes and source-alpha blending with depth disabled.
    material = bytearray(96)
    field(material, 0, INLINE)
    material[72], material[74] = 1, 1   # texture and state-bits counts
    field(material, 84, INLINE)
    field(material, 92, INLINE)
    texture = bytearray(12)
    field(texture, 8, INLINE)
    image = bytearray(32)
    struct.pack_into("<HHH", image, 20, size, size, 1)
    field(image, 28, INLINE)
    name = cstr("qa_glyphs")
    state_bits = words(0x18000165, 2)
    material_body = material + name + texture + image + name + state_bits
    font = words(INLINE, 6, 96, INLINE, 0, INLINE)
    font_body = font + cstr("fonts/smallfont") + material_body + glyphs

    # IWi v8, one A8 mip. The production image reader expands it to white/alpha.
    header = bytearray(32)
    header[:4] = b"IWi\x08"
    header[8] = 5
    struct.pack_into("<HHHII", header, 10, size, size, 1, len(header) + len(pixels), len(header))
    return font_body, header + pixels


def write_fixture(output):
    menus = source_menus()
    font, atlas = bitmap_font()
    assets = [(0x18, font)] + [(0x1a, menu_record(name, items)) for name, items in menus.items()]
    # XFile block lengths, then the inline XAsset list, typed slots and bodies.
    image = bytearray(words(0, 0, 8192, 0, 0, 256 * 1024, 0, 0, 0, 0))
    image.extend(words(0, 0, len(assets), INLINE))
    for kind, _ in assets:
        image.extend(words(kind, INLINE))
    for _, body in assets:
        image.extend(body)
    fastfile = b"IWffu100" + words(0x114) + bytes(9) + zlib.compress(image)

    output.mkdir(parents=True, exist_ok=False)
    zone = output / "zone/english"
    zone.mkdir(parents=True)
    (zone / "common_mp.ff").write_bytes(fastfile)
    main = output / "main"
    main.mkdir()
    with zipfile.ZipFile(main / "fixture.iwd", "x") as archive:
        entry = zipfile.ZipInfo("images/qa_glyphs.iwi", (1980, 1, 1, 0, 0, 0))
        archive.writestr(entry, atlas, compress_type=zipfile.ZIP_DEFLATED)
    print(f"Generated {len(menus)} menu records and a bitmap font in {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new fixture directory, such as /tmp/qa/mw2")
    write_fixture(parser.parse_args().output)
