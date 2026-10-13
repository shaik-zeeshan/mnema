// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here (log-filter.test.ts
// precedent).
import { describe, expect, it } from "bun:test";
import {
	errorTitle,
	filterCounts,
	jobMessage,
	jobProvider,
	jobState,
	matchesFilter,
	nextAttemptLabel,
	subjectLabel,
	timelineTarget,
} from "./jobs";

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

	// The parked state is the whole VISIBLE difference between the old behaviour
	// (a job against an absent model fails three times in ~6 minutes and loses the
	// transcript) and claim-time gating. Without it a parked job is
	// indistinguishable from one that is about to run.
	it("calls a queued job whose model is locked 'preparing'", () => {
		expect(jobState({ ...job("queued", null), modelLocked: true }, NOW)).toBe("preparing");
	});

	// Precedence matters: a parked job may also carry a stale backoff from before
	// its model went away. The model is the reason it will not run next, so that
	// is what the row must say.
	it("prefers 'preparing' over 'retrying' when a parked job also has a backoff", () => {
		expect(jobState({ ...job("queued", "2026-07-15 14:36:00"), modelLocked: true }, NOW)).toBe(
			"preparing",
		);
	});

	// Only a queued job can be parked. A running/failed/completed row claiming
	// "Preparing" would be a lie about work that has already started or ended.
	it("never calls a non-queued job 'preparing', whatever the lock says", () => {
		for (const status of ["running", "failed", "completed"]) {
			expect(jobState({ ...job(status, null), modelLocked: true }, NOW)).toBe(status);
		}
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

describe("filterCounts", () => {
	it("splits retrying out of the lane's queued count", () => {
		const counts = filterCounts({ queued: 14, retrying: 12, running: 1, completed: 200, failed: 3 });
		expect(counts).toEqual({ all: 218, queued: 2, running: 1, retrying: 12, failed: 3 });
	});
});

describe("matchesFilter", () => {
	it("keeps parked jobs under Queued and retrying ones out of it", () => {
		expect(matchesFilter("preparing", "queued")).toBe(true);
		expect(matchesFilter("retrying", "queued")).toBe(false);
		expect(matchesFilter("retrying", "retrying")).toBe(true);
		expect(matchesFilter("completed", "all")).toBe(true);
	});
});

describe("plain error titles", () => {
	it("names the provider for connectivity failures", () => {
		expect(errorTitle("deepgram request timed out after 30s", "deepgram")).toBe("Deepgram can’t be reached");
		expect(errorTitle("HTTP 503 from upstream", null)).toBe("The provider can’t be reached");
	});
	it("recognises a cut-off recording before anything else", () => {
		expect(errorTitle("decode: moov atom not found (segment 2291)", "deepgram")).toBe("The recording was cut off");
	});
	it("recognises a missing model", () => {
		expect(errorTitle("model parakeet-tdt not installed", null)).toBe("The model isn’t installed");
	});
	it("falls back to a generic title", () => {
		expect(errorTitle("disk I/O error", null)).toBe("This job failed");
		expect(errorTitle(null, null)).toBe("This job failed");
	});
});

describe("jobMessage", () => {
	const row = (o = {}) => ({ status: "queued", nextAttemptAt: null, lastError: null, payloadJson: '{"provider":"deepgram"}', ...o });
	it("counts down a retry and says why", () => {
		const r = row({ nextAttemptAt: "2026-07-15 14:31:42", lastError: "connection timed out" });
		expect(jobMessage(r, "retrying", NOW, 12)).toBe("Next try in 42s · Deepgram can’t be reached");
	});
	it("shows the raw first line for an unknown failure", () => {
		expect(jobMessage(row({ status: "failed", lastError: "disk I/O error\nat foo" }), "failed", NOW, 0)).toBe("disk I/O error");
	});
	it("says what a queued job waits behind", () => {
		expect(jobMessage(row(), "queued", NOW, 12)).toBe("Waiting behind 12 retries");
		expect(jobMessage(row(), "queued", NOW, 0)).toBe("Waiting its turn");
		expect(jobMessage(row({ status: "running" }), "running", NOW, 0)).toBeNull();
	});
});

describe("subject + timeline target", () => {
	it("labels and targets frames and segments", () => {
		expect(subjectLabel({ subjectType: "frame", subjectId: 918244 })).toBe("Frame #918244");
		expect(subjectLabel({ subjectType: "audio_segment", subjectId: 2291 })).toBe("Segment #2291");
		expect(timelineTarget({ subjectType: "audio_segment", subjectId: 2291 })).toEqual({ kind: "audio", frameId: null, audioSegmentId: 2291 });
		expect(timelineTarget({ subjectType: "frame", subjectId: 7 })).toEqual({ kind: "frame", frameId: 7, audioSegmentId: null });
		expect(timelineTarget({ subjectType: "other", subjectId: 7 })).toBeNull();
	});
});

describe("jobProvider", () => {
	it("reads the provider out of the payload", () => {
		expect(jobProvider('{"provider":"deepgram","modelId":"nova-3"}')).toBe("deepgram");
	});

	it("is null for absent/empty/invalid payloads — absence, not error", () => {
		expect(jobProvider(null)).toBeNull();
		expect(jobProvider("")).toBeNull();
		expect(jobProvider("{}")).toBeNull();
		expect(jobProvider('{"provider":""}')).toBeNull();
		expect(jobProvider('{"provider":42}')).toBeNull();
		expect(jobProvider("not json")).toBeNull();
	});
});
