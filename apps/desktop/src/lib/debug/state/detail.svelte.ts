// The pushed feature-detail view (level 2): its navigation state, its own
// poller, and the jobs page behind it.
//
// ── Navigation ────────────────────────────────────────────────────────────
// A detail view is a pushed view *within* the /debug page, not a route: the
// shell renders either the summary scroll or the detail, and this store's
// `feature` is the whole of that decision. Nav state lives on the debug
// controller (like every other debug store) because the push comes from a
// summary card's title, deep inside the section tree.
//
// ── Polling ───────────────────────────────────────────────────────────────
// A detail view polls ITS OWN feature only, and the shell stops the summary's
// pollers while one is open — otherwise ten cards' worth of round-trips keep
// running behind a view that renders none of them. So this store re-reads the
// two things its feature actually needs rather than borrowing the summary's
// `features` tick: the pipeline aggregate (for the hero's lane counts) and its
// processor's jobs page — or, for a feature with no job lane, the semantic
// index. The dock's `get_debug_health` poll is the one that keeps running: the
// dock stays on screen, and the hero's diagnosis sentence is its `reason`.

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import { laneFor } from "./features.svelte";
import { DETAIL_SPECS, type DetailFeatureId } from "../detail/specs";
import type {
	ListProcessingJobsRequest,
	ProcessingJobListing,
	ProcessingJobStatus,
	ProcessorPipelineStatus,
	SemanticIndexStatusDto,
} from "$lib/types";

const POLL_MS = 1000;

/** Rows per jobs page. `list_processing_jobs_by_processor` clamps to 0..=500. */
export const JOBS_PAGE_SIZE = 25;

export type DetailTab = "overview" | "jobs" | "config" | "log";

export function createDetailStore() {
	let feature = $state<DetailFeatureId | null>(null);
	let tab = $state<DetailTab>("overview");

	/** `null` = the "all" chip. Filtering is server-side, so it repages. */
	let statusFilter = $state<ProcessingJobStatus | null>(null);
	let page = $state(0);
	let jobs = $state<ProcessingJobListing[]>([]);
	let selectedJobId = $state<number | null>(null);

	let lane = $state<ProcessorPipelineStatus | null>(null);
	let semanticIndex = $state<SemanticIndexStatusDto | null>(null);
	let error = $state<string | null>(null);

	let acting = $state(false);
	let actionMessage = $state<string | null>(null);

	/** Ticks with the poll so "next attempt in 4m 12s" actually counts down. */
	let now = $state(Date.now());

	function open(next: DetailFeatureId) {
		feature = next;
		tab = "overview";
		statusFilter = null;
		page = 0;
		jobs = [];
		selectedJobId = null;
		lane = null;
		semanticIndex = null;
		error = null;
		actionMessage = null;
		void poll();
	}

	function close() {
		feature = null;
	}

	async function poll() {
		const current = feature;
		if (!current) return;
		// Nothing to poll into an off-screen view.
		if (typeof document !== "undefined" && document.visibilityState !== "visible") return;
		now = Date.now();
		const spec = DETAIL_SPECS[current];
		const requestedPage = page;
		const requestedStatus = statusFilter;
		try {
			if (spec.processor) {
				const request: ListProcessingJobsRequest = {
					processor: spec.processor,
					status: requestedStatus,
					limit: JOBS_PAGE_SIZE,
					offset: requestedPage * JOBS_PAGE_SIZE,
				};
				const [lanes, listing] = await Promise.all([
					invoke<ProcessorPipelineStatus[]>("get_processing_pipeline_status"),
					invoke<ProcessingJobListing[]>("list_processing_jobs_by_processor", { request }),
				]);
				// Drilled elsewhere / repaged while this was in flight — that request
				// owns the view now, so drop this one rather than paint a stale page.
				if (feature !== current || page !== requestedPage || statusFilter !== requestedStatus) return;
				lane = laneFor(lanes, spec.processor);
				jobs = listing;
			} else {
				const index = await invoke<SemanticIndexStatusDto>("get_semantic_index_status");
				if (feature !== current) return;
				semanticIndex = index;
			}
			error = null;
		} catch (err) {
			error = humanizeError(err);
		}
	}

	/** 1s poll while a detail view is open + visible. Returns teardown. */
	function startPolling(): () => void {
		void poll();
		const interval = setInterval(() => { void poll(); }, POLL_MS);
		const onVisibility = () => {
			if (document.visibilityState === "visible") void poll();
		};
		document.addEventListener("visibilitychange", onVisibility);
		return () => {
			clearInterval(interval);
			document.removeEventListener("visibilitychange", onVisibility);
		};
	}

	/** The selected row, re-read from the live page so it stays fresh. */
	const selectedJob = $derived(jobs.find((job) => job.id === selectedJobId) ?? null);

	/**
	 * Requeue one job's subject through the reprocess command that already
	 * exists for this processor. No bulk variant: no bulk command exists.
	 */
	async function reprocessSelected() {
		const current = feature;
		const job = selectedJob;
		if (!current || !job || acting) return;
		const spec = DETAIL_SPECS[current];
		// A processor whose jobs don't carry the subject type its reprocess command
		// takes would silently requeue the wrong row's id — refuse instead.
		if (!spec.reprocess || job.subjectType !== spec.subjectType) return;
		acting = true;
		actionMessage = null;
		try {
			await invoke(spec.reprocess.command, { request: { [spec.reprocess.arg]: job.subjectId } });
			actionMessage = `requeued ${job.subjectType} #${job.subjectId}`;
			await poll();
		} catch (err) {
			actionMessage = humanizeError(err);
		} finally {
			acting = false;
		}
	}

	return {
		get feature() { return feature; },
		get isOpen() { return feature !== null; },

		get tab() { return tab; },
		set tab(next: DetailTab) { tab = next; },

		get statusFilter() { return statusFilter; },
		set statusFilter(next: ProcessingJobStatus | null) {
			if (next === statusFilter) return;
			statusFilter = next;
			// A filter is a different list; page 1 of it is the only honest landing.
			page = 0;
			void poll();
		},

		get page() { return page; },
		set page(next: number) {
			const clamped = Math.max(0, next);
			if (clamped === page) return;
			page = clamped;
			void poll();
		},

		get jobs() { return jobs; },
		get selectedJobId() { return selectedJobId; },
		set selectedJobId(next: number | null) { selectedJobId = next; },
		get selectedJob() { return selectedJob; },

		get lane() { return lane; },
		get semanticIndex() { return semanticIndex; },
		get error() { return error; },
		get now() { return now; },

		get acting() { return acting; },
		get actionMessage() { return actionMessage; },
		set actionMessage(next: string | null) { actionMessage = next; },

		open,
		close,
		poll,
		startPolling,
		reprocessSelected,
	};
}

export type DetailStore = ReturnType<typeof createDetailStore>;
