# Add secret redaction for derived text

Mnema will add a **Secret Redaction Pipeline** as downstream mitigation for likely secrets in searchable derived text, applying redaction before search, snippets, copy-text actions backed by OCR or transcripts, and agent-facing derived-text access. It is not capture prevention, media redaction, or secure erase, and it must not be described as protecting original frame, video, or audio media.

## Consequences

V1 targets high-confidence secrets such as API keys, access tokens, private keys, seed-like secrets, structurally obvious passwords, clearly labeled or formatted auth codes, and credential-bearing database connection strings. It uses deterministic high-confidence secret detection rather than broad probabilistic PII model classification, and does not attempt broad PII, name, email, address, phone, sensitive-business-text, screenshot-region, or image redaction. Redaction runs before persistence of searchable derived text from OCR, microphone transcription, and system-audio transcription, so raw secret-bearing OCR or transcript text is not stored by default. If redaction fails before persistence, Mnema fails closed by not persisting raw text as searchable or broker-visible content.

Redaction is always on for searchable and broker-visible derived text once shipped, with no user-facing disable in V1.

Original capture media may still contain content that was redacted from OCR text, audio transcripts, snippets, search projections, copy-text actions, or brokered agent responses. **Delete Recent Capture** remains the recovery path for removing matching media from Mnema's app library, while redaction status should use non-content-bearing markers rather than preserving secret snippets in history or diagnostics. UI surfaces that open, preview, copy from, or export original media associated with redaction metadata should warn that original capture may still contain redacted secrets.

Redaction metadata may include spans, categories, detector versions, and aggregate counts against the redacted text, but not original matched secret values or detector explanations containing those values.
