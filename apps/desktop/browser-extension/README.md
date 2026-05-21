# Mnema Browser Extension

Local WebExtension source for Browser-Integrated Sensitive Capture V2.

The secure-entry channel sends only structure state:

- browser family
- active, clear, or unavailable state
- fixed reason
- observed timestamp
- sequence

It must not send field values, labels, placeholders, selectors, form actions, tab IDs, frame IDs, URLs, domains, titles, screenshots, OCR, or media-derived data on the safety channel.

Metadata is a separate channel and may send active-tab URL only when Mnema metadata settings allow it.

Development flow:

1. Load this directory as an unpacked extension in a supported browser.
2. Pair from Mnema Privacy settings.
3. The native host bridge is intentionally a thin transport; pause/resume decisions stay in Rust.
