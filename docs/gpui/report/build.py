#!/usr/bin/env python3
"""Build the GPUI progress report (anatomy theme, self-contained) from data.json.

    python3 docs/gpui/report/build.py            # -> target/gpui-report/index.html
    npx -y postplan@latest upload target/gpui-report/index.html   # same path = same URL

Screenshots come from apps/desktop-gpui/tests/visual/baselines, shrunk to JPEG and
embedded as data: URIs. Needs Pillow.
"""
import base64, html, io, json, os, sys
from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../.."))
HERE = os.path.dirname(os.path.abspath(__file__))
THEME = os.path.expanduser("~/.claude/skills/postplan/theme")
BASELINES = os.path.join(ROOT, "apps/desktop-gpui/tests/visual/baselines")
OUT = os.path.join(ROOT, "target/gpui-report/index.html")
E = html.escape


def shot(name):
    path = os.path.join(BASELINES, name + ".png")
    if not os.path.exists(path):
        return None
    im = Image.open(path).convert("RGB")
    w = 1100
    im = im.resize((w, int(im.height * w / im.width)), Image.LANCZOS)
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=80, optimize=True)
    return "data:image/jpeg;base64," + base64.b64encode(buf.getvalue()).decode()


def section(n, title):
    return f'<h2><span class="pp-n">{n:02d}</span>{E(title)}</h2>'


def build(d):
    p = []
    p.append('<div class="pp-top"><div><p class="pp-kicker">Mnema · GPUI desktop app · progress</p>'
             '<h1>Building the native Mnema app</h1></div>'
             '<button type="button" class="pp-button" data-theme-toggle aria-label="Switch light or dark theme">Dark</button></div>')
    p.append('<p class="pp-lede">A native GPUI rebuild of the Mnema desktop app that matches today\'s app '
             'pixel for pixel, on a shared Rust backend. This page is updated after every finished step.</p>')
    p.append(f'<p class="pp-meta">Updated {E(d["updated"])} · branch new-gpui-desktop-app</p>')
    p.append(f'<div class="pp-verdict"><span class="pp-chip" data-tone="strong">Status</span><p>{E(d["verdict"])}</p></div>')

    p.append('<div class="pp-metrics">' + "".join(
        f'<div class="pp-metric"><span>{E(m["label"])}</span><b>{E(m["value"])}</b><small>{E(m["note"])}</small></div>'
        for m in d["metrics"]) + "</div>")
    p.extend(f'<div class="pp-bar"><span>{E(b["label"])}</span><i><span style="width:{b["pct"]}%"></span></i>'
             f'<output>{b["pct"]}%</output></div>' for b in d["bars"])

    n = 1
    p.append(section(n, "Happening now")); n += 1
    p.append('<ol class="pp-steps">' + "".join(
        f'<li data-now><span class="pp-sn">▸</span><span class="pp-st"><b>{E(x["title"])}</b>'
        f'<small>{E(x["detail"])}</small></span><span class="pp-chip" data-tone="strong">{E(x["id"])}</span></li>'
        for x in d["now"]) + "</ol>")

    p.append(section(n, "Done so far")); n += 1
    p.append('<table><thead><tr><th>Step</th><th>What it gave us</th><th>Commit</th></tr></thead><tbody>' + "".join(
        f'<tr><td><span class="pp-chip" data-tone="good">{E(x["id"])}</span><br><b>{E(x["title"])}</b></td>'
        f'<td>{E(x["what"])}</td><td><code>{E(x["commit"])}</code></td></tr>'
        for x in reversed(d["done"])) + "</tbody></table>")

    p.append(section(n, "Screenshots")); n += 1
    p.append('<p>The approved reference images every later change is checked against. Dark on the left, light on the right.</p>')
    for g in d["shots"]:
        p.append(f'<h3>{E(g["title"])}</h3><p>{E(g["note"])}</p>')
        for f in g["files"]:
            dark, light = shot(f + "-dark"), shot(f + "-light")
            if not (dark or light):
                continue
            label = f.split("-", 1)[1]
            cells = "".join(
                f'<figure class="shot"><img alt="{E(label)} {t}" src="{src}"><figcaption>{E(label)} · {t}</figcaption></figure>'
                for t, src in (("dark", dark), ("light", light)) if src)
            p.append(f'<div class="shots">{cells}</div>')

    p.append(section(n, "What comes next")); n += 1
    p.append('<ol class="pp-steps">' + "".join(
        f'<li><span class="pp-sn">{i}</span><span class="pp-st"><b>{E(x["title"])}</b><small>{E(x["detail"])}</small></span>'
        f'<span class="pp-chip">{E(x["id"])}</span></li>' for i, x in enumerate(d["next"], 1)) + "</ol>")

    p.append(section(n, "Your decisions so far")); n += 1
    p.append("<ul>" + "".join(f"<li>{E(r)}</li>" for r in d["rulings"]) + "</ul>")

    p.append(section(n, "Risks being watched")); n += 1
    p.append("<ul>" + "".join(f"<li>{E(r)}</li>" for r in d["risks"]) + "</ul>")

    p.append('<footer class="pp-footer">Generated with Claude Code</footer>')
    return "\n".join(p)


def main():
    d = json.load(open(os.path.join(HERE, "data.json")))
    tpl = open(os.path.join(THEME, "template.html")).read()
    css = open(os.path.join(THEME, "theme.css")).read() + """
.shots{display:grid;grid-template-columns:1fr 1fr;gap:12px;margin:10px 0 18px}
.shot{margin:0}.shot img{width:100%;height:auto;display:block;border-radius:8px;border:1px solid var(--anatomy-faint)}
.shot figcaption{font-size:12px;opacity:.7;margin-top:4px}
@media (max-width:640px){.shots{grid-template-columns:1fr}}
"""
    start, end = tpl.index('<article class="pp-doc">') + len('<article class="pp-doc">'), tpl.index("</article>")
    out = tpl[:start] + "\n" + build(d) + "\n" + tpl[end:]
    out = out.replace("/* REPLACE THIS COMMENT WITH THE FULL CONTENTS OF theme.css (inline it; never link it) */", css)
    out = out.replace("<title>Page title</title>", "<title>Mnema GPUI progress</title>")
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    open(OUT, "w").write(out)
    print(OUT, f"{os.path.getsize(OUT) / 1e6:.1f} MB")


if __name__ == "__main__":
    sys.exit(main())
