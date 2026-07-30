#!/usr/bin/env python3
"""Assemble revision-2.html from revision-2.src.html.

Every `<!--PART:slug-->` marker is replaced with the whole of
`input-components/parts/<slug>.part.html`, so the screens carry the real,
live input components rather than a redrawing of them. A part used on a second
screen gets every id renamed (the parts' own isolation rule makes that a
two-line rewrite), so both copies stay alive on one page.

ponytail: substitution, not a bundler — the parts are already self-contained.
"""
import pathlib
import re

HERE = pathlib.Path(__file__).parent
PARTS = HERE / "input-components" / "parts"
SUFFIX = "abcdefgh"

seen: dict[str, int] = {}


def inline(match: re.Match[str]) -> str:
    slug = match.group(1)
    text = (PARTS / f"{slug}.part.html").read_text()
    n = seen.get(slug, 0)
    seen[slug] = n + 1
    if n:
        # `cmp-<slug>` first: renaming it to `cmp-<slug>b` leaves no `<slug>-`
        # for the second pass to hit twice.
        s = SUFFIX[n]
        text = text.replace(f"cmp-{slug}", f"cmp-{slug}{s}")
        text = text.replace(f"{slug}-", f"{slug}{s}-")
    return text


# The section tab strip, one source, four screens — `<!--TABS:n-->` marks which
# section the screen is.
SECTIONS = [
    ("s05a", "What Mnema will do"),
    ("s05b", "Engines"),
    ("s05c", "Models"),
    ("s05d", "AI features"),
]


def tabs(match: re.Match[str]) -> str:
    n = int(match.group(1))
    links = "".join(
        '\n          <a{} href="#{}">{}</a>'.format(' class="on"' if i == n else "", sid, label)
        for i, (sid, label) in enumerate(SECTIONS, start=1)
    )
    return (
        f'<div class="tabs">{links}\n'
        f'          <span class="count">{n} / {len(SECTIONS)}</span>\n        </div>'
    )


src = (HERE / "revision-2.src.html").read_text()
out = re.sub(r"<!--TABS:(\d)-->", tabs, src)
out = re.sub(r"<!--PART:([a-z]+)-->", inline, out)
(HERE / "revision-2.html").write_text(out)
print(f"wrote {HERE / 'revision-2.html'} ({len(out)} bytes, parts: {seen})")
