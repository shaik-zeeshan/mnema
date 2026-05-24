# Product

## Register

brand

## Users

Privacy-conscious power users, knowledge workers, and developers on macOS who want a searchable record of everything they have seen and heard, without handing it to the cloud. They reach the site while trying to retrieve something they half-remember (a document, a conversation, a snippet of code, a face) and care deeply about keeping that record on their own machine. State of mind: curious about the "perfect memory" promise, but instinctively skeptical about what such an app does with their data. The site has to earn trust in the same breath that it sells capability.

## Product Purpose

Mnema is a local-first macOS app that continuously captures screen, audio, and activity into a private, searchable personal record. It exists so you can instantly recall anything you have seen or heard, with capture, OCR, transcription, and storage all happening on-device. The marketing site's job: make a visitor understand "total recall, fully private, on my machine" within one fold, and download the macOS build. Capability is the lead; privacy is the reassurance that closes the deal.

## Brand Personality

Calm, exact, uncanny. The voice is precise and quietly confident, never hyped. It evokes the wonder of perfect memory (you can scrub back through your own past) grounded by the safety of total privacy (none of it leaves your machine). It speaks like a well-built instrument: understated, trustworthy, a little awe-inspiring in what it can do. Three words: calm, exact, uncanny.

## Anti-references

- **Generic SaaS.** Gradient blobs, Inter, an endless grid of identical icon-heading-text feature cards, the big-number "hero metric" template.
- **Consumer-Apple soft.** Pastel, rounded-everything, over-polished and safe (the Rewind.ai look).
- **Crypto / AI hype.** Glowing neon gradients, fake 3D, "revolutionary"/"supercharge" language, loud green-on-black as a hype move.
- **Corporate / enterprise.** Navy and grey, office stock photography, stiff and buttoned-up.

The product is green-on-black like a terminal, so the live risk is sliding into the crypto-hype lane. The resolution: the "drench" is darkness and depth, not glow; green is a precise, meaningful signal (a search match, a live-capture pulse), never decorative neon.

## Design Principles

- **Show the recall, do not claim it.** Demonstrate search and timeline-scrub with a live, privacy-safe artifact instead of adjectives. The interface is the argument.
- **Privacy is proven, not promised.** Lead with concrete mechanisms (on-device OCR/transcription, local SQLite, secret redaction). Practice it too: the site self-hosts its fonts and ships no third-party trackers, so it never phones home about a visitor either.
- **Calm confidence over hype.** No "revolutionary," no exclamation marks, no glow for glow's sake. Precision and restraint carry the trust; the capability is impressive enough stated plainly.
- **The machine records exactly; the human reads easily.** Monospace carries the record voice (labels, timestamps, the UI mock, search queries); a humanist sans carries the human voice (headlines, prose). The split is the concept made visible.
- **Identity continuity.** The site must feel like the app: same terminal/green DNA, same restraint, so download feels like stepping into something already familiar.

## Accessibility & Inclusion

Target WCAG 2.1 AA. Respect `prefers-reduced-motion`: the animated timeline/search mock degrades to a meaningful static frame. Never rely on green alone to convey meaning (pair it with text, icon, or position) so the colorblind and the green-on-black contrast both hold. Maintain AA contrast in both dark and light themes. Fully keyboard-navigable with visible focus. Honor `prefers-color-scheme` for the initial theme, with a manual override.
