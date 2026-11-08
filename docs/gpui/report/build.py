#!/usr/bin/env python3
"""Build the GPUI progress report (anatomy theme, self-contained) from data.json.

    python3 docs/gpui/report/build.py --publish   # build every page and upload it to postplan

Postplan caps a page at 4 MiB, so screenshots live on one page per group
(target/gpui-report/shots-<slug>.html), linked from index.html. Draft ids are kept
in drafts.json so every page keeps its URL.

Screenshots come from apps/desktop-gpui/tests/visual/baselines, shrunk to JPEG and
embedded as data: URIs. Needs Pillow.
"""
import base64, html, io, json, os, re, subprocess, sys
from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../.."))
HERE = os.path.dirname(os.path.abspath(__file__))
THEME = os.path.expanduser("~/.claude/skills/postplan/theme")
BASELINES = os.path.join(ROOT, "apps/desktop-gpui/tests/visual/baselines")
OUT_DIR = os.path.join(ROOT, "target/gpui-report")
DRAFTS = os.path.join(HERE, "drafts.json")
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


def slug(g):
    return re.sub(r"[^a-z0-9]+", "-", g["title"].split(" — ")[0].lower()).strip("-")


def shots_page(g, d):
    p = [f'<div class="pp-top"><div><p class="pp-kicker">Mnema · GPUI desktop app · screenshots</p><h1>{E(g["title"])}</h1></div>'
         '<button type="button" class="pp-button" data-theme-toggle aria-label="Switch light or dark theme">Dark</button></div>',
         f'<p class="pp-lede">{E(g["note"])} Dark on the left, light on the right.</p>',
         f'<p class="pp-meta">Updated {E(d["updated"])}</p>']
    for f in g["files"]:
        dark, light = shot(f + "-dark"), shot(f + "-light")
        if not (dark or light):
            continue
        label = f.split("-", 1)[1]
        cells = "".join(
            f'<figure class="shot"><img alt="{E(label)} {t}" src="{src}"><figcaption>{E(label)} · {t}</figcaption></figure>'
            for t, src in (("dark", dark), ("light", light)) if src)
        p.append(f'<h3>{E(label)}</h3><div class="shots">{cells}</div>')
    p.append('<footer class="pp-footer">Generated with Claude Code</footer>')
    return "\n".join(p)


def build(d, drafts):
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
    p.append('<p>One page per screen (dark next to light). Approved sets are the reference every later change is checked against.</p>')
    p.append('<table><thead><tr><th>Screen</th><th>Status</th><th>Page</th></tr></thead><tbody>' + "".join(
        f'<tr><td><b>{E(g["title"].split(" — ")[0])}</b><br><small>{E(g["note"])}</small></td>'
        f'<td>{E(g["title"].split(" — ")[1]) if " — " in g["title"] else ""}</td>'
        f'<td>' + (f'<a href="{E(drafts[slug(g)]["url"])}">open</a>' if slug(g) in drafts else "publishing…") + '</td></tr>'
        for g in d["shots"]) + "</tbody></table>")

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


def page(tpl, css, title, body):
    start, end = tpl.index('<article class="pp-doc">') + len('<article class="pp-doc">'), tpl.index("</article>")
    out = tpl[:start] + "\n" + body + "\n" + tpl[end:]
    out = out.replace("/* REPLACE THIS COMMENT WITH THE FULL CONTENTS OF theme.css (inline it; never link it) */", css)
    return out.replace("<title>Page title</title>", f"<title>{E(title)}</title>")


def upload(path, draft, description):
    cmd = ["npx", "-y", "postplan@latest", "upload", path] + (["--draft", draft] if draft else ["--description", description])
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    m = re.search(r"Draft ID:\s*(\S+)", out)
    if not m:
        sys.exit(f"upload failed for {path}:\n" + "\n".join(l for l in out.splitlines() if "pp_" not in l))
    return m.group(1)


def main():
    publish = "--publish" in sys.argv
    d = json.load(open(os.path.join(HERE, "data.json")))
    drafts = json.load(open(DRAFTS)) if os.path.exists(DRAFTS) else {}
    tpl = open(os.path.join(THEME, "template.html")).read()
    css = open(os.path.join(THEME, "theme.css")).read() + """
.shots{display:grid;grid-template-columns:1fr 1fr;gap:12px;margin:10px 0 18px}
.shot{margin:0}.shot img{width:100%;height:auto;display:block;border-radius:8px;border:1px solid var(--anatomy-faint)}
.shot figcaption{font-size:12px;opacity:.7;margin-top:4px}
@media (max-width:640px){.shots{grid-template-columns:1fr}}
"""
    os.makedirs(OUT_DIR, exist_ok=True)
    for g in d["shots"]:
        path = os.path.join(OUT_DIR, f"shots-{slug(g)}.html")
        open(path, "w").write(page(tpl, css, g["title"].split(" — ")[0] + " screenshots", shots_page(g, d)))
        print(path, f"{os.path.getsize(path) / 1e6:.1f} MB")
        if publish:
            draft = upload(path, drafts.get(slug(g), {}).get("id"), f"Mnema GPUI — {g['title'].split(' — ')[0]} screenshots")
            drafts[slug(g)] = {"id": draft, "url": f"https://{draft}.postplan.dev"}
    index = os.path.join(OUT_DIR, "index.html")
    open(index, "w").write(page(tpl, css, "Mnema GPUI progress", build(d, drafts)))
    print(index, f"{os.path.getsize(index) / 1e6:.1f} MB")
    if publish:
        drafts["index"] = {"id": upload(index, drafts.get("index", {}).get("id"), "Mnema GPUI app — build progress"), "url": ""}
        drafts["index"]["url"] = f"https://{drafts['index']['id']}.postplan.dev"
        json.dump(drafts, open(DRAFTS, "w"), indent=2)
        print("published", drafts["index"]["url"])


if __name__ == "__main__":
    sys.exit(main())
