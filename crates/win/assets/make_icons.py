"""Draw the NexType icons: a text cursor followed by faded ghost text on a rounded square.

    python crates/win/assets/make_icons.py

Writes nextype.ico (on, indigo) and nextype-off.ico (paused, gray) next to this file,
each with 16-256 px images. Each size is drawn at 8x and downsampled so small sizes stay crisp.
"""
from pathlib import Path

from PIL import Image, ImageDraw

SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
SS = 8  # supersampling factor
HERE = Path(__file__).parent


def draw(size: int, bg: tuple) -> Image.Image:
    s = size * SS
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    u = s / 16  # one unit = one pixel at 16 px
    d.rounded_rectangle([0, 0, s - 1, s - 1], radius=3.5 * u, fill=bg)

    white = (255, 255, 255, 255)
    ghost = (255, 255, 255, 120)
    # Text cursor (I-beam): stem plus serifs.
    cx, top, bot = 5.0 * u, 3.0 * u, 13.0 * u
    stem = 1.5 * u
    d.rectangle([cx - stem / 2, top, cx + stem / 2, bot], fill=white)
    serif = 1.5 * u
    d.rectangle([cx - serif - stem / 2, top, cx + serif + stem / 2, top + 1.2 * u], fill=white)
    d.rectangle([cx - serif - stem / 2, bot - 1.2 * u, cx + serif + stem / 2, bot], fill=white)
    # Ghost text after the caret: two faded word bars on the cursor's middle line.
    y0, y1 = 7.2 * u, 8.8 * u
    r = (y1 - y0) / 2
    d.rounded_rectangle([7.6 * u, y0, 9.8 * u, y1], radius=r, fill=ghost)
    d.rounded_rectangle([10.8 * u, y0, 13.2 * u, y1], radius=r, fill=ghost)
    return img.resize((size, size), Image.LANCZOS)


def write(name: str, bg: tuple) -> None:
    imgs = [draw(n, bg) for n in SIZES]
    big = imgs[-1]
    big.save(HERE / name, format="ICO", sizes=[(n, n) for n in SIZES], append_images=imgs[:-1])
    big.save(HERE / name.replace(".ico", ".png"))


write("nextype.ico", (79, 70, 229, 255))  # indigo
write("nextype-off.ico", (107, 114, 128, 255))  # gray: completions off
