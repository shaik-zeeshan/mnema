// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here (log-filter.test.ts
// precedent).
import { describe, expect, it } from "bun:test";
import { hasNextPage, jobState, nextAttemptLabel, pageRangeLabel } from "./jobs";

// SQLite's naive-UTC shape, exactly as `next_attempt_at` is written
// (`datetime(CURRENT_TIMESTAMP, '+300 seconds')`).
const NOW = Date.parse("2026-07-15T14:31:00Z");
const job = (status: string, nextAttemptAt: string | null = null) => ({ status, nextAttemptAt });

describe("jobState", () => {
	it("passes the three unambiguous wire statuses straight through", () => {
		expect(jobState(job("running"), NOW)).toBe("running");
		expect(jobState(job("completed"), NOW)).toBe("completed");
		// Terminal: the retry lane reverts to `queued`, so `failed` never retries.
		expect(jobState(job("failed", "2026-07-15 14:36:00"), NOW)).toBe("failed");
	});

	it("calls a queued job with a future next attempt 'retrying'", () => {
		expect(jobState(job("queued", "2026-07-15 14:36:00"), NOW)).toBe("retrying");
	});

	it("is plain queued once the backoff has elapsed — it is claimable now", () => {
		expect(jobState(job("queued", "2026-07-15 14:30:00"), NOW)).toBe("queued");
	});

	it("is plain queued when no attempt was ever scheduled", () => {
		expect(jobState(job("queued", null), NOW)).toBe("queued");
	});
});

describe("nextAttemptLabel", () => {
	it("is null when nothing is scheduled — absence, not zero", () => {
		expect(nextAttemptLabel(null, NOW)).toBeNull();
		expect(nextAttemptLabel("nonsense", NOW)).toBeNull();
	});

	it("counts down a future backoff", () => {
		expect(nextAttemptLabel("2026-07-15 14:36:12", NOW)).toBe("in 5m 12s");
		expect(nextAttemptLabel("2026-07-15 14:31:30", NOW)).toBe("in 30s");
	});

	it("says due now once elapsed rather than counting up", () => {
		expect(nextAttemptLabel("2026-07-15 14:30:00", NOW)).toBe("due now");
	});
});

describe("pagination without a total", () => {
	it("offers next only on a full page — a short page is the end", () => {
		expect(hasNextPage(25, 25)).toBe(true);
		expect(hasNextPage(24, 25)).toBe(false);
		expect(hasNextPage(0, 25)).toBe(false);
	});

	it("labels the range it can source, never a total", () => {
		expect(pageRangeLabel(0, 25, 25)).toBe("jobs 1–25");
		expect(pageRangeLabel(1, 4, 25)).toBe("jobs 26–29");
		expect(pageRangeLabel(0, 0, 25)).toBe("no jobs");
		expect(pageRangeLabel(2, 0, 25)).toBe("no more jobs");
	});
});
