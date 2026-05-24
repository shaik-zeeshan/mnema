# Design

Visual system for the Mnema marketing site. Inherits the desktop app's terminal/green identity (the inherited token values are the source of truth, quoted as hex) and extends it for a brand surface. New/brand-only decisions are expressed in OKLCH per the design laws; OKLCH values for inherited colors are approximate conversions of the app hex.

## Theme

Dark by default; light available via manual toggle and `prefers-color-scheme`.

Scene sentence (forces dark): *someone late at night, in a dim room, trying to recall a thing they saw last Tuesday, scrubbing back through their own screen-history while a green match-highlight surfaces the exact moment.* Focused, a little uncanny, private. Dark.

Color strategy: **Committed-dark.** The surface is a deep, slightly-tinted near-black (depth is the drench). A single committed green accent carries meaning, not decoration: it marks a search match, a live-capture pulse, the "found" moment. Sparing by rule, so it never tips into neon hype.

## Color

Inherited dark (source of truth = app hex):

| Role | Hex | OKLCH (approx) |
| --- | --- | --- |
| Background | `#0c0c0e` | `oklch(0.17 0.005 160)` |
| Surface / raised | `#14141a` | `oklch(0.20 0.006 160)` |
| Foreground | `#e2e2e8` | `oklch(0.91 0.004 160)` |
| Foreground muted | `#8a8aaa` | `oklch(0.62 0.02 280)` |
| Border | `#1e1e2e` | `oklch(0.24 0.01 280)` |
| Border strong | `#2a2a3a` | `oklch(0.29 0.012 280)` |
| Accent (green) | `#3dffa0` | `oklch(0.87 0.20 158)` |
| Accent strong | `#2a8a60` | `oklch(0.58 0.12 160)` |
| Accent bg (tint) | `#0d1f15` | `oklch(0.22 0.04 160)` |
| Accent border | `#1a4a30` | `oklch(0.37 0.07 160)` |
| Accent glow | `rgba(61,255,160,0.18)` | for shadows/halos only |
| Danger | `#ff6b7a` | `oklch(0.72 0.17 18)` |
| Info | `#60b0ff` | `oklch(0.74 0.13 250)` |

Inherited light (source of truth = app hex):

| Role | Hex |
| --- | --- |
| Background | `#f6f6f4` |
| Foreground | `#14141a` |
| Foreground muted | `#5a5a6a` |
| Accent (green) | `#1f7a4a` |
| Accent strong | `#155a36` |
| Accent bg | `#e6f4ec` |
| Border | `#d8d8d4` |

Rules:
- Neutrals tint toward the green hue at very low chroma (0.005–0.01). Never `#000`/`#fff`.
- Green usage budget ~10%: kickers, match-highlights, the live dot, one primary CTA, focus rings. Not large fills.
- Glow (`accent-glow`) is reserved for the live-capture pulse and the primary CTA hover, never ambient page decoration.

## Typography

Two self-hosted families (no Google Fonts CDN: privacy + perf, on-brand). Both shipped via `@fontsource` so the site makes no third-party font requests.

- **Mono (the record voice): Spline Sans Mono.** Calm, lightly humanist, precise; not a reflex pick (avoids IBM Plex Mono / Space Mono). Used for kickers/eyebrows, labels, timestamps, nav metadata, the UI mock, search queries, code. Fallback chain matches the app exactly: `"Spline Sans Mono", ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace`.
- **Sans (the human voice): Hanken Grotesk.** Calm humanist grotesque for headlines and body prose; avoids Inter/DM/Plus-Jakarta. Fallback: `"Hanken Grotesk", system-ui, sans-serif`.

(If a chosen family is not trivially installable via fontsource at build time, the fallbacks above are themselves on-brand: system mono is the app's literal stack.)

Headlines are sans (calm, confident), preceded by a small green mono kicker. Mono never carries long headlines (avoids "mono as costume"); it stays in its earned label/record role.

Scale: modular, fluid `clamp()` for headings, ≥1.25 ratio between steps. Body 16–18px, line length capped 65–75ch, line-height +0.05–0.1 on dark.

## Components

- **Mono kicker:** small uppercase-ish green mono label with a leading `//` or a live dot; sits above section headlines.
- **Primary CTA:** "Download for macOS" — solid green on dark, subtle glow on hover only. Single per fold.
- **Secondary CTA:** ghost/outline, mono label (e.g. "View source").
- **Timeline/search mock (hero centerpiece):** privacy-safe, code-built recreation of the app: a scrubber with time ticks, a search field, result rows with green match-highlights, a live-capture dot. Animated; static frame under reduced-motion.
- **Spec rows over cards:** capability list rendered as a mono "index"/spec sheet (label + value + description) rather than identical icon cards. Cards only where genuinely the best affordance; never nested, never a uniform grid of them.

## Layout

- Left-aligned, asymmetric compositions over centered stacks. A visible structural grid (terminal/spec aesthetic) is welcome as voice.
- Fluid spacing with `clamp()`; vary rhythm (generous section separation, tight intra-group). No uniform padding everywhere.
- Single dominant idea per fold; long, deliberately paced scroll.
- Section art-direction may shift: the privacy section can read calmer/more contained than the capability sections, as long as the voice holds.

## Motion

- Ease-out exponential curves (ease-out-quart/quint/expo). No bounce, no elastic.
- One orchestrated page-load with staggered reveals; restraint elsewhere.
- The hero mock animates the core idea (scrub → match surfaces in green). Never animate layout properties; transition transform/opacity. Expand/collapse via `grid-template-rows`.
- Everything motion respects `prefers-reduced-motion`.
