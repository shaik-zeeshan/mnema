# Cloud transcription errors are transient liveness, not job failures

## Status

Accepted.

## Context

The processing-job queue has a binary run outcome — `Completed` or `Failed` — and every failure consumes one bounded retry attempt (`crates/app-infra/src/retry_policy.rs`) before the job is left terminally failed. That contract is sound for local providers, whose availability is gated at admission (model installed or not) and which rarely fail transiently at run time. A cloud provider inverts this: the common failures are environmental (machine offline, timeout, rate limit, server error, rejected key) and strike *after* admission. Under pure reuse, a weekend offline with Deepgram selected would burn every queued job's attempt cap and leave those segments permanently untranscribed — silent data loss dressed as retry semantics. ADR 0021 (display-unavailable) and ADR 0040 (low disk) already establish the house pattern: an environmental "can't proceed right now" condition is transient liveness, not failure.

## Decision

The Deepgram provider classifies errors, and the queue treats the two classes differently:

- **Connectivity-shaped** (offline, timeout, HTTP 429/5xx) **and auth-shaped** (HTTP 401/403) errors are transient liveness: the job **requeues with backoff without incrementing its failure count**, and the segment waits indefinitely — exactly like a segment waiting for a model download. This adds one store operation (requeue-without-counting) to the processing store.
- **Segment-specific rejections** (the vendor says *this* audio is malformed/unsupported) are genuine failures: existing bounded retry, terminal after the attempt cap. Retrying a corrupt file forever helps nobody.
- **A rejected key additionally surfaces in the Settings transcription panel** ("Deepgram rejected your API key"), because liveness-requeued jobs are silent by design and a revoked key must not look identical to "everything's fine".

## Considered Options

- **Pure reuse of bounded retries for all errors.** Rejected: converts ordinary offline stretches into permanent transcription loss for every segment recorded during them.
- **Admission-time reachability probes** (only enqueue when the network/vendor looks up). Rejected: racy and unownable; availability at admission stays defined as "key present" only, and run-time classification handles the rest.
- **Unbounded retries for everything.** Rejected: a genuinely malformed segment would grind the queue forever; the bounded path must survive for per-segment defects.

## Consequences

- Error classification lives in the provider implementation (it is vendor-specific HTTP knowledge); the queue only learns "liveness requeue" vs "genuine failure".
- The liveness requeue reuses the existing saturating backoff schedule — no new tuning constants.
- The queue can now grow without terminal failures while offline or mis-keyed; the Settings surfacing is the pressure gauge, since nothing else makes that state visible.
