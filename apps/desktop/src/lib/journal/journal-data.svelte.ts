// The Journal destination's one reader.
//
// Journal is not a third surface (direction 02 README, bullet 6): it is a
// destination opened from Overview's Today tile, and it re-uses the machinery
// the old Insights Journal already had — `buildJournalDay` for the day model,
// `journal-view` for banding, `lede-stats` for the four honest figures. This
// class is only the loading half of `lib/insights/DayTimeline.svelte`, lifted
// into a rune class so the studio page can stay layout.
//
// Every figure obeys **G8**: a read that fails leaves its slice empty and the
// surface drops that line rather than showing a zero.

import { invoke } from "@tauri-apps/api/core";
import type {
	Activity,
	AiRuntimeStatus,
	Conclusion,
	UserContextDigest,
	UserContextStatus,
} from "$lib/types/recording";
import type { FrameSummaryDto } from "$lib/types/app-infra";
import { dayKeyOf, dayWindow } from "$lib/overview/overview-format";

/** A day step can cost a model call (a fresh range misses the digest cache), so
 *  stepping through days debounces the digest read. Lifted from DayTimeline. */
const DIGEST_DEBOUNCE_MS = 500;

export class JournalData {
	/** The local day the river is about. */
	dayKey = $state(dayKeyOf(new Date()));

	activities = $state<Activity[]>([]);
	frames = $state<FrameSummaryDto[]>([]);
	/** Distilled beliefs — the inspector's "subjects touched" row reads their
	 *  activity-evidence refs. One standing read, not per selection. */
	conclusions = $state<Conclusion[]>([]);
	usage = $state<{ timePerApp: { activeMs: number }[] } | null>(null);

	aiStatus = $state<AiRuntimeStatus | null>(null);
	ctxStatus = $state<UserContextStatus | null>(null);
	statusLoaded = $state(false);

	digest = $state<UserContextDigest | null>(null);
	digestLoading = $state(false);
	digestRegenerating = $state(false);
	digestError = $state<string | null>(null);

	/** False until the first activities+frames read lands (drives the skeleton). */
	rangeLoadedOnce = $state(false);
	usageLoaded = $state(false);

	engineOn = $derived(
		Boolean(this.aiStatus?.enabled && this.aiStatus?.available) ||
			Boolean(this.ctxStatus?.engineAvailable),
	);

	readonly window = $derived(dayWindow(this.dayKey));

	#rangeToken = 0;
	#usageToken = 0;
	#digestToken = 0;
	#regenSeq = 0;
	#digestTimer: ReturnType<typeof setTimeout> | null = null;

	setDay(key: string): void {
		if (key === this.dayKey) return;
		this.dayKey = key;
		this.rangeLoadedOnce = false;
		this.usageLoaded = false;
		void this.loadRange();
		void this.loadUsage();
		this.#scheduleDigestLoad();
	}

	/** Mount / live-event refresh: statuses first (the digest read gates on them). */
	async reloadAll(): Promise<void> {
		await this.loadStatus();
		await Promise.all([this.loadRange(), this.loadUsage(), this.loadDigest(), this.loadConclusions()]);
	}

	/** In-place refresh on a worker beat — never resets `rangeLoadedOnce`, so the
	 *  river updates without blanking to a skeleton on every derivation. */
	async refresh(): Promise<void> {
		await this.loadStatus();
		await Promise.all([this.loadRange(), this.loadUsage(), this.loadDigest()]);
	}

	dispose(): void {
		if (this.#digestTimer != null) clearTimeout(this.#digestTimer);
		this.#digestTimer = null;
	}

	async loadStatus(): Promise<void> {
		const [ai, ctx] = await Promise.all([
			invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
			invoke<UserContextStatus>("get_user_context_status").catch(() => null),
		]);
		this.aiStatus = ai;
		this.ctxStatus = ctx;
		this.statusLoaded = true;
	}

	async loadRange(): Promise<void> {
		const token = ++this.#rangeToken;
		const { startMs, endMs } = this.window;
		try {
			const [activities, frames] = await Promise.all([
				invoke<Activity[]>("list_user_context_activities", { startMs, endMs }),
				invoke<FrameSummaryDto[]>("list_frame_summaries_in_range", {
					request: {
						capturedAtStart: new Date(startMs).toISOString(),
						capturedAtEnd: new Date(endMs).toISOString(),
					},
				}),
			]);
			if (token !== this.#rangeToken) return; // the day moved on
			this.activities = activities;
			this.frames = frames;
		} catch {
			// Best-effort: a failed read leaves the previous river standing.
		} finally {
			if (token === this.#rangeToken) this.rangeLoadedOnce = true;
		}
	}

	async loadUsage(): Promise<void> {
		const token = ++this.#usageToken;
		const { startMs, endMs } = this.window;
		try {
			const next = await invoke<{ timePerApp: { activeMs: number }[] }>("get_usage_charts", {
				startMs,
				endMs,
			});
			if (token !== this.#usageToken) return;
			this.usage = next;
		} catch {
			if (token === this.#usageToken) this.usage = null;
		} finally {
			if (token === this.#usageToken) this.usageLoaded = true;
		}
	}

	async loadConclusions(): Promise<void> {
		try {
			this.conclusions = await invoke<Conclusion[]>("list_user_context_conclusions", {
				includeFaded: false,
			});
		} catch {
			this.conclusions = [];
		}
	}

	async loadDigest(): Promise<void> {
		if (!this.statusLoaded || !this.engineOn) {
			this.digest = null;
			this.digestLoading = false;
			return;
		}
		const token = ++this.#digestToken;
		this.digestLoading = true;
		this.digestError = null;
		try {
			const { startMs, endMs } = this.window;
			const next = await invoke<UserContextDigest | null>("get_user_context_digest", {
				rangeKind: "day",
				startMs,
				endMs,
			});
			if (token !== this.#digestToken) return;
			this.digest = next;
		} catch {
			if (token === this.#digestToken) this.digest = null;
		} finally {
			if (token === this.#digestToken) this.digestLoading = false;
		}
	}

	/** The tool strip's "↻ re-read". Its own sequence for the busy flag, because
	 *  `#digestToken` is bumped by every worker-beat refresh (DayTimeline's bug
	 *  note: a token-gated reset leaves the button stuck on "reading…"). */
	async regenerateDigest(): Promise<void> {
		if (!this.engineOn || this.digestRegenerating) return;
		const token = ++this.#digestToken;
		const regen = ++this.#regenSeq;
		this.digestRegenerating = true;
		this.digestLoading = false;
		this.digestError = null;
		try {
			const { startMs, endMs } = this.window;
			const next = await invoke<UserContextDigest | null>("regenerate_user_context_digest", {
				rangeKind: "day",
				startMs,
				endMs,
			});
			if (token !== this.#digestToken) return;
			this.digest = next;
			if (!next) this.digestError = "Not enough activity in this day to write a read.";
		} catch (error) {
			if (token === this.#digestToken) {
				this.digestError = error instanceof Error ? error.message : "Couldn't write a read.";
			}
		} finally {
			if (regen === this.#regenSeq) this.digestRegenerating = false;
		}
	}

	#scheduleDigestLoad(): void {
		this.#digestToken += 1; // drop anything queued for the old day
		this.digest = null;
		this.digestRegenerating = false;
		this.digestError = null;
		if (this.#digestTimer != null) clearTimeout(this.#digestTimer);
		this.#digestTimer = null;
		if (!this.statusLoaded || !this.engineOn) {
			this.digestLoading = false;
			return;
		}
		this.digestLoading = true; // the placeholder spans the debounce window too
		this.#digestTimer = setTimeout(() => {
			this.#digestTimer = null;
			void this.loadDigest();
		}, DIGEST_DEBOUNCE_MS);
	}
}
