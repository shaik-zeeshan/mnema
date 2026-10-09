# Secret Redaction V2 uses a deterministic derived-text gate

Mnema will implement **Secret Redaction Pipeline** V2 as an always-on deterministic **Secret Redaction Gate** for derived text, not as ML classification, live capture prevention, media redaction, or asynchronous cleanup. V2 broadens high-confidence secret detection through a staged, bounded scanner, adds OCR-aware visual-line and normalization handling with conservative over-redaction when span mapping is uncertain, keeps transcript redaction on shared high-confidence detector families, and fails processing completion closed when no safe redacted persistence plan can be produced.

This preserves a clear privacy boundary: searchable, copyable, snippet, and broker-visible derived text is admitted only after redaction succeeds, while original media may still contain secrets and remains governed by warning and **Delete Recent Capture** recovery flows.

## Amendment (2026-10-09): payment card numbers are secrets

Detector v3 (`secret-redaction-v3`) adds the `payment_card` category, marker `[REDACTED_SECRET: PAYMENT_CARD]`. A card number (PAN) is a credential, so it is in scope despite the general broad-PII exclusion.

- PAN acceptance is issuer prefix plus length plus Luhn plus separator sanity (every digit group but the last has at least four digits) — never Luhn alone, which passes about one random digit string in ten.
- The PAN detector scans the full surface (the number's format is its own evidence). Each match is refined to the longest validating contiguous sub-run of digit groups, so an expiry year, CVV, or order number joined onto the same OCR visual line neither masks the card number nor gets swallowed into its redaction.
- A card number next to its label (`card number`, `card no`, `card #`) is redacted from 8 digits on, without PAN validation: narrow checkout fields show only a prefix (e.g. 12 of 16 digits), which no PAN check can accept but which still leaks most of the card.
- OCR forms that stack a label above its value are paired across the two visual lines, so the value observation is redacted too, not just the joined result text.
- An OCR visual-line hit now replaces only the observations the match actually touches, not every observation on the line: the line is the observations' space-join, so the mapping is exact, and on wide checkout rows the old whole-line unit hid prices and help text. Labeled card matches (CVV, labeled card number) are trimmed to their digits, so the label stays readable.
- CVV/CVC is label-gated (`cvv`, `cvc`, `security code`, `card code`, `card verification`; `cid` only inside a window another label opened).
- Out of scope: expiry dates (not secrets alone) and card numbers spoken as number words in transcripts.
- Per [ADR 0015](0015-secret-redaction-v2-is-prospective.md), existing results are not reprocessed; v3 applies only to newly processed text.
