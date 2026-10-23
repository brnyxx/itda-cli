"""Stamps the tile mark from assets/logo.svg into the files that carry a copy.

assets/logo.svg is the one master. The wordmarks keep their letterforms and
take the tile from it; the hero cursor on the landing takes the cursor and
bolt outlines. Run from anywhere:

    python3 assets/src/mark.py

Then `python3 assets/src/flow.py`, which reads the master itself.
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LOGO = ROOT / "assets" / "logo.svg"
MARK = re.compile(r"<!-- mark -->.*?<!-- /mark -->", re.S)


def tile(prefix, edge):
    svg = LOGO.read_text(encoding="utf-8")
    inner = svg[svg.index("<defs>") : svg.rindex("</svg>")]
    inner = inner.replace('id="m-', f'id="{prefix}-').replace("url(#m-", f"url(#{prefix}-")
    inner = re.sub(r'(class="tile"[^>]*stroke-opacity=")[^"]*', rf"\g<1>{edge}", inner)
    return f"<!-- mark -->{inner.strip()}<!-- /mark -->"


def outline(cls):
    match = re.search(rf'class="{cls}" d="([^"]+)"', LOGO.read_text(encoding="utf-8"))
    return match.group(1)


def stamp(path, replacement):
    text = path.read_text(encoding="utf-8")
    if not MARK.search(text):
        raise SystemExit(f"{path}: no <!-- mark --> ... <!-- /mark --> to stamp into")
    path.write_text(MARK.sub(lambda _: replacement, text, count=1), encoding="utf-8")


def main():
    stamp(ROOT / "assets" / "wordmark-dark.svg", tile("lk", ".14"))
    stamp(ROOT / "assets" / "wordmark-light.svg", tile("lk", ".08"))
    index = ROOT / "site" / "index.html"
    html = index.read_text(encoding="utf-8")
    for cls, target in (("cursor", "hc-arrow"), ("bolt", "hc-bolt")):
        html = re.sub(rf'(id="{target}" d=")[^"]*', rf"\g<1>{outline(cls)}", html)
    index.write_text(html, encoding="utf-8")


if __name__ == "__main__":
    main()
