# Product

## Register

product

Note: this repo holds two surfaces. The desktop app (`apps/desktop`) is the product register default. The marketing site (`apps/web`) is always **brand** register: design IS the product there.

## Users

- Primary: privacy-conscious Mac users (developers, researchers, knowledge workers) who want perfect recall of their own screen, audio, and activity without sending data to a cloud.
- Site visitors arrive skeptical: "another AI recorder?" They need to feel two things fast: this is genuinely local/private, and the recall experience is powerful enough to be worth running all day.
- App users are in a daily-driver workflow: recording runs quietly, they open Mnema to search, scrub the timeline, or ask AI about their past.

## Product Purpose

Mnema continuously records screen, audio, and activity on the user's Mac, turns it into a searchable, scrubable, AI-queryable memory, and keeps every byte on-device. Success for the marketing site: a visitor understands the Recall + AI story in one scroll, believes the privacy claim, and downloads.

## Brand Personality

Terminal-native, calm, precise, quietly confident. "A machine that remembers, working for you in the background." Green-on-dark terminal identity with a strict type split: sans (Hanken Grotesk) = human voice, mono (Spline Sans Mono) = machine voice (labels, kickers, data, UI chrome). The redesign direction (confirmed 2026-07-07): keep this identity and elevate it hard — bigger type, bento showcase sections with live-feeling mini-demos, varied section rhythm with full-bleed moments, smooth scroll-reveal motion, one or two signature visual effects. Ambitious, not loud.

## Anti-references

- Generic SaaS landing pages: cream/lavender gradients, identical icon-card grids, hero-metric templates, gradient text.
- Crypto/neon hacker excess: the terminal look must stay calm and legible, never edgy.
- Rewind/limitless-style cloud-AI marketing that hides where data goes. Mnema leads with on-device.
- Jargon on the landing page: all technical/CLI/agent detail lives on /agents only. Landing copy stays human.

## Design Principles

1. Show the memory, don't describe it: sections should feel like live captures (search results, transcripts, timeline scrubs) rather than claims.
2. Machine voice for machine things: mono type, statuses, and timestamps carry the terminal identity; human copy stays warm sans.
3. Privacy is a design feature: "on-device" appears as ambient, verifiable-feeling UI (status dots, local paths), not a badge wall.
4. Rhythm over repetition: vary section width, density, and pace; no two adjacent sections with the same layout skeleton.
5. Motion earns its place: reveals and scrubs reinforce "recall/rewind", always reduced-motion safe.

## Accessibility & Inclusion

- Respect `prefers-reduced-motion` everywhere (already a site-wide kill switch; keep it working with any new motion system).
- Both dark and light themes are first-class; every new surface must read correctly in both.
- Keep text contrast high on the dark theme; the green accent is never used for long body text.
- Semantic headings and focus-visible outlines stay intact.
