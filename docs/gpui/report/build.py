#!/usr/bin/env python3
"""Build the GPUI progress report from data.json, on the postplan anatomy theme.

    python3 docs/gpui/report/build.py            # write target/gpui-report/*.html
    python3 docs/gpui/report/build.py --publish  # ...and upload each page to postplan

Every page starts from ~/.claude/skills/postplan/theme/template.html with the whole of theme.css
inlined, and uses only the theme's own components. Postplan caps a page at 4 MiB, so screenshots
live on one page per screen group (shots-<slug>.html) plus one page for today's fixes
(fixes.html), all linked from index.html. drafts.json keeps each page's draft id, so URLs never change.

Screenshots come from apps/desktop-gpui/tests/visual/baselines (shrunk to JPEG); the fixes page
reads the JPEGs in docs/gpui/report/fixes. Everything is embedded as data: URIs. Needs Pillow.
"""
import base64, html, io, json, os, re, subprocess, sys
from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../.."))
HERE = os.path.dirname(os.path.abspath(__file__))
THEME = os.path.expanduser("~/.claude/skills/postplan/theme")
BASELINES = os.path.join(ROOT, "apps/desktop-gpui/tests/visual/baselines")
FIXES = os.path.join(HERE, "fixes")
OUT_DIR = os.path.join(ROOT, "target/gpui-report")
DRAFTS = os.path.join(HERE, "drafts.json")
CAP = 4 * 1024 * 1024
E = html.escape


# ---------- pictures ----------

def baseline(name):
    path = os.path.join(BASELINES, name + ".png")
    if not os.path.exists(path):
        return None
    im = Image.open(path).convert("RGB")
    im = im.resize((1100, int(im.height * 1100 / im.width)), Image.LANCZOS)
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=80, optimize=True)
    return "data:image/jpeg;base64," + base64.b64encode(buf.getvalue()).decode()


def fix_image(name):
    with open(os.path.join(FIXES, name), "rb") as f:
        return "data:image/jpeg;base64," + base64.b64encode(f.read()).decode()


def figure(src, caption):
    return (f'<figure class="pp-shot"><img alt="{E(caption)}" src="{src}">'
            f'<figcaption>{E(caption)}</figcaption></figure>')


def row(figs):
    """One figure full width, or several side by side to compare."""
    return figs[0] if len(figs) == 1 else '<div class="pp-grid">' + "".join(figs) + "</div>"


# ---------- page parts ----------

def top(kicker, title, lede, meta):
    return (f'<div class="pp-top"><div><p class="pp-kicker">{E(kicker)}</p><h1>{E(title)}</h1></div>'
            '<button type="button" class="pp-button" data-theme-toggle aria-label="Switch light or dark theme">Dark</button></div>'
            f'<p class="pp-lede">{lede}</p><p class="pp-meta">{meta}</p>')


def h2(n, title, anchor=None, toc=None):
    attrs = (f' id="{anchor}"' if anchor else "") + (f' data-toc="{E(toc)}"' if toc else "")
    return f'<h2{attrs}><span class="pp-n">{n:02d}</span>{E(title)}</h2>'


def page_chip(d, key):
    """A page's status from its own sections, so the index never says "awaiting" when nothing is."""
    secs = d.get(key, [])
    if any(s.get("awaiting") for s in secs):
        return chip("awaiting your approval", "strong")
    if any("in progress" in s.get("state", "") for s in secs):
        return chip("in progress", "strong")
    return chip("nothing waiting")


def chip(text, tone=None):
    tone_attr = f' data-tone="{tone}"' if tone else ""
    return f'<span class="pp-chip"{tone_attr}>{E(text)}</span>'


def steps(items, now=False):
    return '<ol class="pp-steps">' + "".join(
        f'<li{" data-now" if now else ""}><span class="pp-sn">{i}</span><span class="pp-st"><b>{E(x["title"])}</b>'
        f'<small>{E(x["detail"])}</small></span>{chip(x["id"], "strong" if now else None)}</li>'
        for i, x in enumerate(items, 1)) + "</ol>"


def link(drafts, key, text, anchor=""):
    return f'<a href="{E(drafts[key]["url"])}{anchor}">{E(text)}</a>' if key in drafts else E(text)


def code(text):
    return "—" if text == "—" else f"<code>{E(text)}</code>"


FOOTER = '<footer class="pp-footer">Generated with Claude Code</footer>'


def slug(g):
    return re.sub(r"[^a-z0-9]+", "-", g["title"].split(" — ")[0].lower()).strip("-")


# ---------- pages ----------

def shots_page(g, d, drafts):
    name = g["title"].split(" — ")[0]
    p = [top("Mnema · GPUI desktop app · screenshots", name,
             E(g["note"]) + " Dark on the left, light on the right.",
             f'Updated {E(d["updated"])} · {link(drafts, "index", "back to the progress report")}')]
    pending = g.get("pending", {})
    if pending:
        p.append(f'<div class="pp-callout"><p><b>{len(pending)} of these screens changed on 10 Nov,</b> and a new Models screenshot was added. '
                 'The pictures here are still the approved versions; the new ones wait for your approval. '
                 f'See them side by side on {link(drafts, "fixes", "Today’s fixes: before and after", "#fix-onboarding")}.</p></div>')
    n = 0
    for f in g["files"]:
        dark, light = baseline(f + "-dark"), baseline(f + "-light")
        if not (dark or light):
            continue
        n += 1
        label = f.split("-", 1)[1]
        p.append(h2(n, label))
        if f in pending:
            p.append(f'<p>{chip("changed · awaiting approval", "strong")} {E(pending[f])}</p>')
        p.append(row([figure(src, f"{label} · {t}") for t, src in (("dark", dark), ("light", light)) if src]))
    p.append(FOOTER)
    return "\n".join(p)


def fixes_page(d, drafts, key="fixes", title="Today’s fixes: before and after",
               lede="Pictures of each fix from your hands-on test. None of the changed screenshots replace the approved "
                    "ones until you approve them, so this is the only place they show."):
    p = [top("Mnema · GPUI desktop app · " + ("11 Nov 2026" if key in ("round4", "round5", "round6") else "10 Nov 2026"), title, lede,
             f'Updated {E(d["updated"])} · {link(drafts, "index", "back to the progress report")}'),
         '<div class="pp-verdict"><span class="pp-chip" data-tone="strong">Your call</span>'
         f'<p>{E(d[key + "_verdict"])}</p></div>']
    for n, x in enumerate(d[key], 1):
        p.append(h2(n, x["title"], x["anchor"], x.get("toc")))
        p.append(f'<p>{chip(x["state"], "strong" if x.get("awaiting") else None)} '
                 f'<span class="pp-meta">commit <code>{E(x["commit"])}</code></span></p>')
        p.append(f'<p>{E(x["what"])}</p>')
        if x.get("how"):
            p.append(f'<div class="pp-callout"><p>{E(x["how"])}</p></div>')
        for part in x.get("rows", []):
            if part.get("heading"):
                p.append(f'<h3>{E(part["heading"])}</h3>')
            p.append(row([figure(fix_image(s["file"]), s["caption"]) for s in part["shots"]]))
    p.append(FOOTER)
    return "\n".join(p)


def index_page(d, drafts):
    p = [top("Mnema · GPUI desktop app · progress", "Building the native Mnema app",
             "A native GPUI rebuild of the Mnema desktop app that matches today’s app pixel for pixel, "
             "on a shared Rust backend. This page is updated after every finished step.",
             f'Updated {E(d["updated"])} · branch new-gpui-desktop-app'),
         f'<div class="pp-verdict"><span class="pp-chip" data-tone="strong">Status</span><p>{E(d["verdict"])}</p></div>',
         '<div class="pp-metrics">' + "".join(
             f'<div class="pp-metric"><span>{E(m["label"])}</span><b>{E(m["value"])}</b><small>{E(m["note"])}</small></div>'
             for m in d["metrics"]) + "</div>"]
    p.extend(f'<div class="pp-bar"><span>{E(b["label"])}</span><i><span style="width:{b["pct"]}%"></span></i>'
             f'<output>{b["pct"]}%</output></div>' for b in d["bars"])

    n = 1
    p.append(h2(n, "Your six issues from today’s hands-on test", toc="Today’s six issues")); n += 1
    p.append(f'<p>{E(d["words"])}</p>')
    p.append('<table><thead><tr><th>Issue</th><th>State</th><th>Commit</th><th>Picture</th></tr></thead><tbody>' + "".join(
        f'<tr><td><b>{E(x["title"])}</b><br><small>{E(x["note"])}</small></td>'
        f'<td>{chip(x["state"], "strong" if x.get("awaiting") else None)}</td><td>{code(x["commit"])}</td>'
        f'<td>{link(drafts, "fixes", "before / after", "#" + x["anchor"]) if x.get("anchor") else "—"}</td></tr>'
        for x in d["issues"]) + "</tbody></table>")

    p.append(h2(n, "Happening now")); n += 1
    p.append(steps(d["now"], now=True))

    p.append(h2(n, "Waiting on you")); n += 1
    p.append(steps(d["waiting"]))

    p.append(h2(n, "Done so far")); n += 1
    p.append('<table><thead><tr><th>Step</th><th>What it gave us</th><th>Commit</th></tr></thead><tbody>' + "".join(
        f'<tr><td>{chip(x["id"])}<br><b>{E(x["title"])}</b></td><td>{E(x["what"])}</td><td><code>{E(x["commit"])}</code></td></tr>'
        for x in reversed(d["done"])) + "</tbody></table>")

    p.append(h2(n, "Screenshots")); n += 1
    p.append('<p>One page per screen, dark next to light. Approved sets are the reference every later change '
             'is checked against. Today’s changed screens are on their own page until you approve them.</p>')
    rows = ([(link(drafts, "round6", "The date control, checked against the Tauri app"), "11 Nov · the Insights calendar's week rows and month grid, Chat's range, the Timeline jumper",
              page_chip(d, "round6"))] if d.get("round6") else []) + ([(link(drafts, "round5", "Settings › Intelligence, checked against the Tauri app"), "11 Nov · the provider Manage panel, connectors, model sections",
              page_chip(d, "round5"))] if d.get("round5") else []) + ([(link(drafts, "round4", "11 Nov hands-on round: fixes"), "11 Nov · menus drawn under the page, copy buttons, missing models, the developer window from the status popover",
              page_chip(d, "round4"))] if d.get("round4") else []) + ([(link(drafts, "round3", "Real-app test and behaviour audit"), "10 Nov night · what works when actually used, what is broken, what the old app does that the new one doesn't",
              page_chip(d, "round3"))] if d.get("round3") else []) + ([(link(drafts, "round2", "Second hands-on round: fixes"), "10 Nov evening · search, text cursor, developer logs, links, scroll bars",
              page_chip(d, "round2"))] if d.get("round2") else []) + ([(link(drafts, "receipts", "Licence receipts: the final design"), "your pick on every licence receipt, with the three original directions below",
              page_chip(d, "receipts"))] if d.get("receipts") else []) + [(link(drafts, "approve", "Earlier approvals"), "screenshots you approved on 10 Nov before they became references",
             page_chip(d, "approve")),
            (link(drafts, "fixes", "Today’s fixes: before and after"), "10 Nov · before and after pictures of each fix",
             chip("built"))]
    rows += [(link(drafts, slug(g), g["title"].split(" — ")[0]), g["note"],
              chip(g["title"].split(" — ")[1], "strong" if g.get("pending") else None)) for g in d["shots"]]
    p.append('<table><thead><tr><th>Page</th><th>Status</th></tr></thead><tbody>' + "".join(
        f'<tr><td><b>{a}</b><br><small>{E(note)}</small></td><td>{c}</td></tr>' for a, note, c in rows) + "</tbody></table>")

    if d.get("checklist"):
        p.append(h2(n, "Your hands-on checklist", toc="Hands-on checklist")); n += 1
        p.append(f'<p>{E(d["checklist"]["intro"])}</p>')
        p.append('<ol class="pp-steps">' + "".join(
            f'<li><span class="pp-sn">{i}</span><span class="pp-st"><b>{E(x["try"])}</b><small>{E(x["right"])}</small></span>'
            f'{chip(x["tag"])}</li>' for i, x in enumerate(d["checklist"]["items"], 1)) + "</ol>")

    p.append(h2(n, "What comes next")); n += 1
    p.append(steps(d["next"]))
    p.append(h2(n, "Your decisions so far", toc="Your decisions")); n += 1
    p.append("<ul>" + "".join(f"<li>{E(r)}</li>" for r in d["rulings"]) + "</ul>")
    p.append(h2(n, "Risks being watched", toc="Risks")); n += 1
    p.append("<ul>" + "".join(f"<li>{E(r)}</li>" for r in d["risks"]) + "</ul>")
    p.append(FOOTER)
    return "\n".join(p)


# ---------- assemble + publish ----------

def page(tpl, css, title, body):
    start = tpl.index('<article class="pp-doc">') + len('<article class="pp-doc">')
    out = tpl[:start] + "\n" + body + "\n" + tpl[tpl.index("</article>"):]
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
    css = open(os.path.join(THEME, "theme.css")).read()
    os.makedirs(OUT_DIR, exist_ok=True)

    def emit(key, name, title, body, description):
        path = os.path.join(OUT_DIR, name)
        open(path, "w").write(page(tpl, css, title, body))
        size = os.path.getsize(path)
        print(path, f"{size / 1048576:.2f} MiB")
        if size > CAP:
            sys.exit(f"{name} is over postplan's 4 MiB cap")
        if publish:
            draft = upload(path, drafts.get(key, {}).get("id"), description)
            drafts[key] = {"id": draft, "url": f"https://{draft}.postplan.dev"}
            json.dump(drafts, open(DRAFTS, "w"), indent=2)

    # sub-pages first, so the index links to their (possibly new) URLs; they link back to the index
    emit("fixes", "fixes.html", "Today’s fixes", fixes_page(d, drafts), "Mnema GPUI — today's fixes, before and after")
    emit("approve", "approve.html", "Earlier approvals",
         fixes_page(d, drafts, "approve", "Earlier approvals",
                    "Screenshots you approved on 10 Nov before they became references. Nothing here is waiting."),
         "Mnema GPUI — earlier approvals")
    if d.get("round6"):
        emit("round6", "round6.html", "Date control check",
             fixes_page(d, drafts, "round6", "The date control, checked against the Tauri app",
                        "The calendar control used in Insights, Chat and the Timeline, compared with the Tauri app view by view. The new-app pictures are rendered from test data; the Tauri pictures are the Tauri app's own render of the same data."),
             "Mnema GPUI — date control check")
    if d.get("round5"):
        emit("round5", "round5.html", "Settings › Intelligence check",
             fixes_page(d, drafts, "round5", "Settings › Intelligence, checked against the Tauri app",
                        "Every control and click-through flow in Settings › Intelligence was listed from the Tauri app's code and matched to the new app. What was missing or wrong is built. The pictures are rendered from test data."),
             "Mnema GPUI — Settings › Intelligence check")
    if d.get("round4"):
        emit("round4", "round4.html", "11 Nov hands-on round",
             fixes_page(d, drafts, "round4", "11 Nov hands-on round: fixes",
                        "What you found using the real dev build on 11 Nov, and what was done about each. The pictures are rendered from test data."),
             "Mnema GPUI — 11 Nov hands-on round")
    if d.get("round3"):
        emit("round3", "round3.html", "Real-app test and behaviour audit",
             fixes_page(d, drafts, "round3", "Real-app test and behaviour audit",
                        "While you were away on 10 Nov: an agent used the real dev build by hand, and another compared what the two apps do behind the scenes. The pictures here are rendered from test data; captures of the real app are not published, because they show your own screen content."),
             "Mnema GPUI — real-app test and behaviour audit")
    if d.get("round2"):
        emit("round2", "round2.html", "Second hands-on round",
             fixes_page(d, drafts, "round2", "Second hands-on round: fixes",
                        "What you found in the clean first run on 10 Nov, and what was done about each."),
             "Mnema GPUI — second hands-on round")
    if d.get("receipts"):
        emit("receipts", "receipts.html", "Licence receipts: pick a direction",
             fixes_page(d, drafts, "receipts", "Licence receipts: pick a direction",
                        "Three redesigned directions for the receipt you see after buying Mnema, each built and rendered in the real app."),
             "Mnema GPUI — licence receipt directions")
    for g in d["shots"]:
        name = g["title"].split(" — ")[0]
        emit(slug(g), f"shots-{slug(g)}.html", f"{name} screenshots", shots_page(g, d, drafts), f"Mnema GPUI — {name} screenshots")
    emit("index", "index.html", "Mnema GPUI progress", index_page(d, drafts), "Mnema GPUI app — build progress")
    if publish:
        print("published", drafts["index"]["url"])


if __name__ == "__main__":
    sys.exit(main())
