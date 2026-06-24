#!/usr/bin/env python3
"""Assemble apps/desktop/THIRD_PARTY_LICENSES.md from cargo-about + license-checker output.

Paths default to repo-relative locations resolved from this script's own
location, so it works regardless of the current working directory. Override any
of them with --rust-in / --js-in / --out (e.g. to feed in temp-dir generator
output during a release build).
"""
import argparse
import html
import json
import re
from collections import defaultdict
from datetime import date
from pathlib import Path

# This script lives at scripts/assemble-third-party-licenses.py;
# the repo root is one level up.
_SCRIPT_DIR = Path(__file__).resolve().parent
_REPO_ROOT = _SCRIPT_DIR.parents[0]
_DEFAULT_OUT = _REPO_ROOT / "apps" / "desktop" / "THIRD_PARTY_LICENSES.md"

_parser = argparse.ArgumentParser(description=__doc__)
_parser.add_argument("--rust-in", default=str(_SCRIPT_DIR / "rust-licenses.txt"),
                     help="cargo-about output (default: alongside this script)")
_parser.add_argument("--js-in", default=str(_SCRIPT_DIR / "js-licenses.json"),
                     help="license-checker JSON (default: alongside this script)")
_parser.add_argument("--out", default=str(_DEFAULT_OUT),
                     help="output Markdown path (default: apps/desktop/THIRD_PARTY_LICENSES.md)")
_args = _parser.parse_args()

RUST_IN = _args.rust_in
JS_IN = _args.js_in
OUT = _args.out


def unesc(s: str) -> str:
    # cargo-about .hbs double-escapes via handlebars; unescape twice to be safe.
    return html.unescape(html.unescape(s))


# ---- Parse Rust blocks -------------------------------------------------------
raw = open(RUST_IN, encoding="utf-8").read()
blocks = raw.split("@@@LICENSE@@@")[1:]
# Map: spdx_id -> {"name": display, "crates": set("name version"), "texts": {normalized_text: original_text}}
rust = {}
for b in blocks:
    header, rest = b.split("\n", 1)
    header = header.strip()
    m = re.match(r"^(.*)\(([^)]+)\)\s*$", header)
    name, spdx = unesc(m.group(1).strip()), m.group(2).strip()
    # rest starts with \n@@@USEDBY@@@ ... @@@TEXT@@@ ... @@@ENDLICENSE@@@
    seg = rest.split("@@@USEDBY@@@", 1)[1]
    usedby_raw, txt_seg = seg.split("@@@TEXT@@@", 1)
    text = txt_seg.split("@@@ENDLICENSE@@@", 1)[0]
    crates = []
    for line in usedby_raw.splitlines():
        line = line.strip()
        if line.startswith("- "):
            crates.append(line[2:].strip())
    text = unesc(text).strip("\n")
    entry = rust.setdefault(spdx, {"name": name, "crates": set(), "texts": {}})
    entry["crates"].update(crates)
    # dedup texts by a whitespace-normalized key, keep the longest original
    key = re.sub(r"\s+", " ", text).strip()
    if key and (key not in entry["texts"] or len(text) > len(entry["texts"][key])):
        entry["texts"][key] = text

# ---- Parse JS ----------------------------------------------------------------
js = json.load(open(JS_IN, encoding="utf-8"))
# group by license string -> list of (pkg, license_text)
js_by_lic = defaultdict(list)
js_pkgs = []
for pkg, meta in sorted(js.items(), key=lambda kv: kv[0].lower()):
    lic = meta.get("licenses", "UNKNOWN")
    if isinstance(lic, list):
        lic = " OR ".join(lic)
    lic = str(lic).rstrip("*")  # license-checker '*' = guessed
    lic_file = meta.get("licenseFile")
    text = None
    if lic_file and lic_file.lower().split("/")[-1].startswith("license"):
        try:
            t = open(lic_file, encoding="utf-8", errors="replace").read().strip()
            # skip if it's actually a README (no license-ish content marker is hard; keep README-derived only if short)
            text = t
        except Exception:
            text = None
    js_pkgs.append((pkg, lic))
    js_by_lic[lic].append((pkg, text))

# ---- Render ------------------------------------------------------------------
rust_crate_count = len({c for e in rust.values() for c in e["crates"]})
js_pkg_count = len(js_pkgs)

LIC_ORDER = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "MPL-2.0",
             "BSL-1.0", "Zlib", "0BSD", "Unicode-3.0", "Unicode-DFS-2016",
             "CC0-1.0", "Unlicense", "NCSA", "bzip2-1.0.6", "CDLA-Permissive-2.0"]


def order_key(spdx):
    return (LIC_ORDER.index(spdx) if spdx in LIC_ORDER else len(LIC_ORDER), spdx)


out = []
w = out.append

w(f"""# Third-Party Licenses and Attribution

_Mnema desktop application — bundled third-party software notices._

This document lists the third-party open-source software bundled in, statically
linked into, or otherwise distributed with the Mnema desktop application, along
with each component's license. Where a license requires that its text be retained
in distributions (MIT, BSD, Apache-2.0, ISC, MPL-2.0, and similar), the full
license text is reproduced here.

It is organized in three parts:

1. **Rust dependencies** — crates compiled into the desktop binary
   (`apps/desktop/src-tauri`, crate `mnema`, and the workspace crates it pulls in),
   grouped by license.
2. **JavaScript / frontend dependencies** — packages bundled into the compiled
   web frontend shipped inside the app, grouped by license.
3. **Bundled native libraries and model assets** — components that are not
   tracked by the package managers (OpenBLAS/LAPACK, the GCC runtime libraries,
   and downloadable ML model weights).

Generated {date.today().isoformat()}. Mnema's own first-party source code is **not**
open source and is **not** covered by this document.

Coverage: **{rust_crate_count} Rust crates**, **{js_pkg_count} JavaScript packages**.

To regenerate this file, see the "Regenerating this file" section at the end.

---

## Part 1 — Rust dependencies

The following crates are compiled into the shipped desktop binary. They are grouped
by SPDX license identifier. Crates offered under a choice of licenses (e.g.
`MIT OR Apache-2.0`) are listed under the license whose text appears below; the
full set of options for each crate remains available from the crate's source.
""")

for spdx in sorted(rust.keys(), key=order_key):
    e = rust[spdx]
    w(f"\n### {e['name']} (`{spdx}`)\n")
    w("Applies to the following Rust crates:\n")
    for c in sorted(e["crates"], key=str.lower):
        w(f"- {c}")
    w("")
    texts = list(e["texts"].values())
    if not texts:
        continue
    if len(texts) == 1:
        w("\n```")
        w(texts[0])
        w("```")
    else:
        w(f"\n_{len(texts)} distinct license-text variants were found for this"
          " license among the crates above (differing copyright lines); all are"
          " reproduced below._\n")
        for i, t in enumerate(texts, 1):
            w(f"\n<details><summary>Variant {i}</summary>\n\n```")
            w(t)
            w("```\n</details>")
    w("")

w("\n---\n\n## Part 2 — JavaScript / frontend dependencies\n")
w("""The compiled web frontend shipped inside the desktop app bundles the
following packages (production dependency closure, enumerated with
`license-checker-rseidelsohn`). Build-only tooling that does not ship in the
bundle may also appear; it is retained for completeness. Packages are grouped by
their declared license.
""")

for lic in sorted(js_by_lic.keys(), key=lambda l: (l.lower())):
    pkgs = js_by_lic[lic]
    w(f"\n### `{lic}`\n")
    w("Applies to the following packages:\n")
    for pkg, _ in pkgs:
        w(f"- {pkg}")
    w("")
    # reproduce up to one representative license text per (license, text) seen
    seen = {}
    for pkg, text in pkgs:
        if not text:
            continue
        key = re.sub(r"\s+", " ", text).strip()[:4000]
        if key not in seen:
            seen[key] = (pkg, text)
    if seen:
        for pkg, text in seen.values():
            w(f"\n<details><summary>License text (as shipped with {pkg})</summary>\n\n```")
            w(text.strip())
            w("```\n</details>")
    w("")

w("""
---

## Part 3 — Bundled native libraries and model assets

These components are not tracked by Cargo or the JavaScript package manager but are
either statically linked into the shipped binary or downloaded on demand at runtime.

### OpenBLAS / LAPACK (BSD-3-Clause)

The on-device speaker-diarization path (`speakrs`) links **OpenBLAS** — including
its bundled Fortran **LAPACK** — built from source and linked statically into the
shipped binary. OpenBLAS and the reference LAPACK it incorporates are distributed
under the **BSD 3-Clause License**.

```
Copyright (c) 2011-2014, The OpenBLAS Project
Copyright (c) 1992-2013 The University of Tennessee and The University of
                        Tennessee Research Foundation. All rights reserved.
Copyright (c) 2000-2013 The University of California Berkeley. All rights reserved.
Copyright (c) 2006-2013 The University of Colorado Denver. All rights reserved.
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.
3. Neither the name of the OpenBLAS project nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### GCC runtime libraries — `libgfortran`, `libquadmath`, `libgcc`

OpenBLAS's Fortran components require the GCC runtime libraries, which are
statically linked into the shipped binary (`libgfortran.a`, `libquadmath.a`,
`libgcc.a`). These libraries are licensed under the **GNU General Public License
version 3 or later, WITH the GCC Runtime Library Exception (version 3.1)**. The
Runtime Library Exception grants permission to convey the resulting binary under
the recipient's chosen terms — it does **not** impose the GPL's copyleft on a
program merely because it is linked with these runtime libraries when compiled
with a GCC "Eligible Compilation Process". The full texts of the GPLv3 and the
GCC Runtime Library Exception are available at:

- https://www.gnu.org/licenses/gpl-3.0.html
- https://www.gnu.org/licenses/gcc-exception-3.1.html

### Machine-learning model weights (downloaded on demand)

Several ML models are downloaded at runtime rather than bundled in the installer.
Models distributed under attribution licenses (notably **CC-BY-4.0** for the
**WeSpeaker** speaker-embedding model and the **Parakeet** transcription model)
carry their attribution in-app via the **Third-Party Notices** screen. That
surface is assembled descriptor-by-descriptor at runtime from each model crate's
manifest; the attribution source of truth lives in
`apps/desktop/src-tauri/src/third_party_notices.rs`. The full text of
CC-BY-4.0 is available at https://creativecommons.org/licenses/by/4.0/legalcode .

---

## Regenerating this file

The Rust portion is produced with [`cargo-about`](https://github.com/EmbarkStudios/cargo-about)
using `apps/desktop/src-tauri/about.toml` and `apps/desktop/src-tauri/about.hbs`:

```sh
# from apps/desktop/src-tauri
cargo about generate about.hbs --frozen > rust-licenses.txt
```

The JavaScript portion is produced with `license-checker-rseidelsohn`:

```sh
# from apps/desktop
bunx license-checker-rseidelsohn --production --json > js-licenses.json
```

The two outputs are then merged into this file by the assembler script kept with
the generation run. Neither tool requires a full compile or the `mnema-cli`
sidecar / OpenBLAS toolchain — both read manifests and `Cargo.lock` only.
""")

open(OUT, "w", encoding="utf-8").write("\n".join(out) + "\n")
print("wrote", OUT)
print("rust crates:", rust_crate_count, "| js packages:", js_pkg_count)
print("rust license groups:", len(rust), "| js license groups:", len(js_by_lic))
