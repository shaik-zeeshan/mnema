# Secret Redaction V2 uses a deterministic derived-text gate

Mnema will implement **Secret Redaction Pipeline** V2 as an always-on deterministic **Secret Redaction Gate** for derived text, not as ML classification, live capture prevention, media redaction, or asynchronous cleanup. V2 broadens high-confidence secret detection through a staged, bounded scanner, adds OCR-aware visual-line and normalization handling with conservative over-redaction when span mapping is uncertain, keeps transcript redaction on shared high-confidence detector families, and fails processing completion closed when no safe redacted persistence plan can be produced.

This preserves a clear privacy boundary: searchable, copyable, snippet, and broker-visible derived text is admitted only after redaction succeeds, while original media may still contain secrets and remains governed by warning and **Delete Recent Capture** recovery flows.
