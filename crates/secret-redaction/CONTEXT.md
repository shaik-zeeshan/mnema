# Secret Redaction Context

Derived-text secret detection and redaction policy for search, copy, snippets, and broker-visible text.

Root entry point: [CONTEXT-MAP.md](../../CONTEXT-MAP.md).

## Language

**Secret Redaction Pipeline**:
A future ADR-backed downstream mitigation flow that detects likely secrets in searchable derived text and withholds or replaces that text before search, snippets, copy-text, or agent-facing access.
_Avoid_: capture prevention, media redaction, secure erase

## Relationships

- **Secret Redaction Pipeline** affects searchable derived text, snippets, copy-text actions backed by OCR or transcripts, and agent-facing derived text access, not original frame, video, or audio media.
- **Secret Redaction Pipeline** V1 targets high-confidence secrets such as API keys, access tokens, private keys, seed-like secrets, structurally obvious passwords, clearly labeled or formatted auth codes, and credential-bearing database connection strings.
- **Secret Redaction Pipeline** V1 does not attempt broad PII, name, email, address, phone, sensitive-business-text, screenshot-region, or image redaction.
- **Secret Redaction Pipeline** V1 uses deterministic high-confidence secret detection rather than broad probabilistic PII model classification.
- **Secret Redaction Pipeline** is always on for searchable and broker-visible derived text once shipped, with no user-facing disable in V1.
- **Secret Redaction Pipeline** runs before persistence of searchable derived text from OCR, microphone transcription, and system-audio transcription, and does not store raw secret-bearing OCR or transcript text by default.
- **Secret Redaction Pipeline** applies to broker-visible or searchable word/token payloads as well as display strings; timing metadata may remain, but original secret token text must not be persisted by default.
- **Secret Redaction Pipeline** may inspect bounded in-memory context around OCR or transcript text to classify high-confidence secrets, but must not persist raw context windows.
- If **Secret Redaction Pipeline** fails before searchable derived text persistence, Mnema must fail closed by not persisting raw text as searchable or broker-visible content.
- **Secret Redaction Pipeline** removes exact secret values from searchability; search may match surrounding non-secret context and redaction categories, but not the original secret value.
- **Secret Redaction Pipeline** may persist redaction spans, categories, detector versions, and aggregate counts against redacted text, but never the original matched secret value.
- User-facing **Secret Redaction Pipeline** metadata should expose coarse categories such as API key, access token, private key, password, auth code, connection string, or seed-like secret, not detector internals, confidence scores, or matched prefixes/suffixes.
- **Secret Redaction Pipeline** reprocessing may add redactions to existing searchable derived text when detectors improve, but it cannot restore original secret values and should not inspect original media by default.
- UI surfaces that open, preview, copy from, or export original media associated with redaction metadata should warn that original capture may still contain redacted secrets.
- Search and derived-text UI may show non-content redaction metadata such as redaction category, count, and `has redactions` filters, but not matched secret values or detector explanations that include secret text.
- Original capture media may still contain redacted content, so **Delete Recent Capture** remains the recovery path for removing media from Mnema's app library.
