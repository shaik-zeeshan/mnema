# Secret Redaction Context

Derived-text secret detection and redaction policy for search, copy, snippets, and broker-visible text.

Root entry point: [CONTEXT-MAP.md](../../CONTEXT-MAP.md).

## Language

**Secret Redaction Pipeline**:
A future ADR-backed downstream mitigation flow that detects likely secrets in searchable derived text and withholds or replaces that text before search, snippets, copy-text, or agent-facing access.
_Avoid_: capture prevention, media redaction, secure erase

**OCR-Aware Secret Redaction**:
The OCR-specific part of Secret Redaction Pipeline V2 that handles visual-line joining, OCR normalization views, and OCR-safe fallback units.
_Avoid_: Secret Redaction Pipeline V2, transcript redaction

**Secret Redaction Gate**:
The synchronous persistence boundary that admits only redacted OCR or transcript derived text into durable derived-text storage.
_Avoid_: async scrubber, post-processing cleanup

**Redaction Safety Failure**:
A terminal processing-job outcome where Mnema cannot prove that derived text is safe to persist after redaction.
_Avoid_: no secret found, OCR miss, empty result

**OCR Redaction Unit**:
A bounded OCR text region Mnema can safely replace when exact secret-span mapping is uncertain.
_Avoid_: crop, screenshot region, image redaction unit

**OCR Visual Line**:
A derived OCR text line formed by joining nearby source observations that appear to belong to the same visible line of text.
_Avoid_: raw OCR observation, paragraph, text block

**Unified Redaction Plan**:
A single safe persistence plan that redacts all searchable, copyable, snippet, and broker-visible derived text for one processing result.
_Avoid_: per-field scrub, display-only redaction

**Redaction Surface**:
The derived-text surface where a redaction was applied, such as top-level result text, an OCR visual line, an OCR observation, a transcript segment, or a transcript word.
_Avoid_: raw field path, JSON pointer, detector evidence

**OCR Normalization View**:
A bounded in-memory alternative view of OCR text, such as separator-normalized or whitespace-collapsed text, used only to find redaction candidates.
_Avoid_: persisted normalized OCR, hidden searchable text copy

**Transcript Redaction**:
Secret redaction over recognized speech text from microphone or system-audio transcription.
_Avoid_: OCR-tolerant redaction, audio censoring, media redaction

**Transcript Redaction Unit**:
A bounded transcript text region Mnema can safely replace when exact word-level mapping is uncertain.
_Avoid_: audio clip, waveform region, media redaction unit

**Staged Redaction Scan**:
A deterministic secret scan that prefilters for likely secret evidence, extracts bounded candidate text, and routes candidates to specific detector families.
_Avoid_: full-text regex sweep, unbounded scan

**Redaction Candidate Window**:
A bounded in-memory text window around label, prefix, provider, or syntax evidence that is safe for detector-specific validation.
_Avoid_: raw context capture, persisted evidence

**Redaction Work Budget**:
The bounded work policy that limits how much derived text the Secret Redaction Gate scans before persistence.
_Avoid_: performance optimization, CPU limit

**Redaction Fixture Suite**:
The detector-policy test corpus that proves expected secret redactions and expected non-redactions for Mnema-derived text.
_Avoid_: sample secrets, benchmark-only data

**Prospective Redaction Guarantee**:
The V2 promise that improved redaction applies to newly completed or explicitly reprocessed derived text, not automatic mutation of existing V1 results.
_Avoid_: retroactive cleanup, history scrub, migration redaction

**Redaction Telemetry**:
Non-content operational data about Secret Redaction Gate behavior.
_Avoid_: detector evidence, matched text, raw context log

## Relationships

- **Secret Redaction Pipeline** affects searchable derived text, snippets, copy-text actions backed by OCR or transcripts, and agent-facing derived text access, not original frame, video, or audio media.
- **Secret Redaction Pipeline** V1 targets high-confidence secrets such as API keys, access tokens, private keys, seed-like secrets, structurally obvious passwords, clearly labeled or formatted auth codes, and credential-bearing database connection strings.
- **Secret Redaction Pipeline** V2 should broaden high-confidence secret coverage to provider-prefixed API keys, generic labeled API keys, access tokens, secret keys, passwords, bearer/auth headers, private keys, JWTs, database/queue/cache connection strings, cloud access keys, webhook URLs with embedded secrets, OAuth/client secrets, auth or verification codes, and seed/recovery phrases.
- **OCR-Aware Secret Redaction** is a capability inside V2, not the umbrella name for V2.
- **Secret Redaction Pipeline** V1 does not attempt broad PII, name, email, address, phone, sensitive-business-text, screenshot-region, or image redaction.
- **Secret Redaction Pipeline** V2 should continue to exclude broad PII, emails, phone numbers, addresses, names, business-sensitive prose, URL/domain privacy heuristics, private-window detection, browser password-page detection, and password-field detection.
- **Secret Redaction Pipeline** V1 uses deterministic high-confidence secret detection rather than broad probabilistic PII model classification.
- **Secret Redaction Pipeline** V2 may use mature secret scanners as references for detector families and fixtures, but shipped detectors should remain Mnema-owned, deterministic, OCR-aware, bounded, and covered by Mnema false-positive and false-negative fixtures.
- **Secret Redaction Pipeline** V2 may use entropy checks only after label, prefix, syntax, or bounded context evidence has produced a suspicious candidate; entropy alone should not broadly redact OCR text except for very strong provider-specific formats.
- **Secret Redaction Pipeline** V2 may fuzzy-match OCR-distorted labels, provider names, and separators, but secret values should be recognized through provider-specific formats, token syntax, bounded entropy after context evidence, or other structural validation rather than fuzzy value matching.
- **Secret Redaction Pipeline** V2 should use a **Staged Redaction Scan** so broad detector coverage does not require every detector to inspect every full derived-text surface.
- **Staged Redaction Scan** should extract **Redaction Candidate Window** values around likely evidence and route each candidate to relevant detector families, validators, and entropy checks.
- Strong, cheap, provider-specific token formats may still scan full derived-text surfaces when they are precise and bounded enough not to undermine the gate's work budget.
- **Redaction Work Budget** should be defined primarily by input size, **Redaction Candidate Window** size, total scanned candidate text, and allowed full-surface detector families, with a short timeout only as a final safety net.
- When **Redaction Work Budget** pressure occurs, **Secret Redaction Gate** should first degrade to conservative redaction of suspicious bounded units and treat the result as a **Redaction Safety Failure** only when no safe bounded redaction unit exists.
- **Redaction Fixture Suite** belongs with the `secret-redaction` context and should cover clean provider tokens, dirty OCR variants, transcript-like phrasing, labeled JSON/env/YAML values, auth headers, connection strings, private keys, JWTs, seed phrases, and false-positive controls such as UUIDs, hashes, CSS colors, stack traces, placeholders, and random IDs.
- App-infra tests should verify **Secret Redaction Gate** integration behavior such as no raw derived-text persistence, structured payload redaction, metadata counts, failed processing jobs on **Redaction Safety Failure**, and search or broker surfaces observing only redacted text.
- **Secret Redaction Gate** may scan **OCR Normalization View** values, but each hit must map back to exact original spans or a safe **OCR Redaction Unit**, and the normalized raw text must not be persisted.
- **Transcript Redaction** uses the shared high-confidence secret detector families, but should not inherit OCR-only visual-line joining, OCR confusion handling, or whitespace-collapsed value reconstruction unless a transcript-specific rule justifies it.
- When **Transcript Redaction** finds likely secret text but exact word-level mapping is uncertain, **Secret Redaction Gate** should redact the smallest safe **Transcript Redaction Unit**, such as the containing transcript segment, before treating the result as a **Redaction Safety Failure**.
- Browser URL metadata that becomes searchable, copyable, snippet, or broker-visible derived text should pass through **Secret Redaction Gate** when included in those surfaces, especially query strings and fragments, but this is not live URL privacy protection or browser password/private-window detection.
- **Secret Redaction Pipeline** is always on for searchable and broker-visible derived text once shipped, with no user-facing disable in V1 or V2.
- V2 should not introduce user-facing detector policy settings such as OCR/transcript redaction toggles or strictness sliders; user-facing controls should be explicit actions such as reprocessing with current redaction or deleting recent capture.
- **Secret Redaction Pipeline** runs before persistence of searchable derived text from OCR, microphone transcription, and system-audio transcription, and does not store raw secret-bearing OCR or transcript text by default.
- **Secret Redaction Gate** runs during processing result completion, after OCR or transcription has produced in-memory raw derived text and before `processing_results`, search projections, copy-text surfaces, snippets, or broker-visible responses can observe that text.
- **Secret Redaction Pipeline** applies to broker-visible or searchable word/token payloads as well as display strings; timing metadata may remain, but original secret token text must not be persisted by default.
- **Secret Redaction Pipeline** may inspect bounded in-memory context around OCR or transcript text to classify high-confidence secrets, but must not persist raw context windows.
- If **Secret Redaction Pipeline** fails before searchable derived text persistence, Mnema must fail closed by not persisting raw text as searchable or broker-visible content.
- If **Secret Redaction Gate** cannot produce a safe persistence plan, the processing job should fail with a sanitized non-content error rather than complete with missing or raw derived text.
- **Secret Redaction Gate** failures should not be automatically retried by the normal processing queue; transient gate failures may be retried through an explicit bounded reprocessing path, while deterministic unsafe span mapping or unsupported payload shape should wait for detector/payload handling changes.
- A **Redaction Safety Failure** is distinct from finding no secret: no-match redaction results are successful gate outcomes, while safety failures mean Mnema could not safely classify, map, or serialize the derived text for persistence.
- **Secret Redaction Gate** should scan **OCR Visual Line** values before raw source observations so labels and values split across observations can still be recognized.
- Redaction-specific **OCR Visual Line** grouping belongs to the `secret-redaction` context as part of **OCR-Aware Secret Redaction**; the `ocr` context should keep provider output structures, and app-infra should stay the persistence integration boundary.
- `secret-redaction` should expose provider-neutral redaction input DTOs, while app-infra adapts OCR and transcript provider payloads into those DTOs before invoking **Secret Redaction Gate**.
- When OCR-tolerant scanning finds likely secret text but exact span mapping is uncertain, **Secret Redaction Gate** should redact the smallest safe **OCR Redaction Unit**, such as an **OCR Visual Line** or source observation, before treating the result as a **Redaction Safety Failure**.
- **Secret Redaction Gate** should produce a **Unified Redaction Plan** across top-level OCR or transcript text and structured OCR/transcript payload fields, rather than treating display text, observations, segments, or words as unrelated redaction passes.
- **Unified Redaction Plan** metadata should count redactions found only in structured payload fields as well as redactions found in top-level result text.
- **Unified Redaction Plan** metadata may persist a coarse **Redaction Surface** and safe redacted-text positions or aggregate counts, but must not persist raw field paths, original secret values, matched prefixes/suffixes, or raw context windows.
- V2 implementation should center on a planner-style API that accepts all relevant derived-text surfaces for one processing result and returns a **Unified Redaction Plan**; simple text-only redaction may remain as a wrapper for narrow callers and tests.
- **Redaction Telemetry** may include aggregate input size buckets, candidate counts, detector-family counts, elapsed time, budget-degraded counts, and redaction counts by coarse category, but must not include raw text, matched values, prefixes/suffixes, context windows, provider-specific token hints, or OCR normalization text.
- **Prospective Redaction Guarantee** versioning is internal or diagnostic; normal search, snippet, copy, and broker surfaces should not add noisy V1/V2 badges for results that have no redaction metadata.
- User-facing **Redaction Safety Failure** copy should say text recognition was not saved because Mnema could not safely redact sensitive text, without exposing raw OCR/transcript text or matched detector details.
- **Secret Redaction Pipeline** removes exact secret values from searchability; search may match surrounding non-secret context and redaction categories, but not the original secret value.
- Redaction category markers may be searchable so users can find redacted-secret results, but original secret values must not remain searchable after **Secret Redaction Gate** succeeds.
- **Secret Redaction Pipeline** may persist redaction spans, categories, detector versions, and aggregate counts against redacted text, but never the original matched secret value.
- User-facing **Secret Redaction Pipeline** metadata should expose coarse categories such as API key, access token, private key, password, auth code, connection string, or seed-like secret, not detector internals, confidence scores, or matched prefixes/suffixes.
- V2 detector families should map provider-specific or syntax-specific detections back to coarse user-facing redaction categories rather than exposing provider names, detector names, confidence scores, or token fragments in markers or metadata.
- V2 should add a user-facing redaction category only when it changes user understanding or recovery behavior; JWTs, bearer tokens, cloud keys, webhook secrets, OAuth secrets, and provider-specific keys should usually collapse into existing API key, access token, password, or connection string categories.
- **Prospective Redaction Guarantee** means existing V1-derived OCR or transcript results remain as-is unless the user explicitly reprocesses the underlying job; V2 does not automatically rerun redaction over historical V1 results.
- Explicit **Secret Redaction Pipeline** reprocessing may add redactions to existing searchable derived text when detectors improve, but it cannot restore original secret values and should not inspect original media by default.
- UI surfaces that open, preview, copy from, or export original media associated with redaction metadata should warn that original capture may still contain redacted secrets.
- **Secret Redaction Pipeline** should not block original media preview, copy, or export merely because redaction metadata exists; it should warn and keep **Delete Recent Capture** as the recovery path.
- Search and derived-text UI may show non-content redaction metadata such as redaction category, count, and `has redactions` filters, but not matched secret values or detector explanations that include secret text.
- Broker-visible responses may expose redacted derived text plus coarse redaction flags, counts, or categories when useful, but must not expose detector internals, matched values, token fragments, raw context windows, or explanations that include secret evidence.
- Original capture media may still contain redacted content, so **Delete Recent Capture** remains the recovery path for removing media from Mnema's app library.
