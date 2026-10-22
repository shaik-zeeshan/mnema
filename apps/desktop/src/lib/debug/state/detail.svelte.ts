// Processor drill-in state (Pipeline › <processor>): the jobs page behind the
// status filter, the inspected job, and the per-job actions.
//
// Navigation is the controller's (`nav.processor` / `nav.jobId`); this store
// only holds data. ProcessorView drives `fetchJobs` through `controller.poll`
// at 2 s while it is mounted. The lanes and the meaning index come from the
// Pipeline tab's own poll, which keeps running behind the drill-in.

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import { DETAIL_SPECS, type ProcessorKey } from "../detail/specs";
import { jobState, matchesFilter, type JobFilter } from "../detail/jobs";
import type {
	ListProcessingJobsRequest,
	ProcessingJobDto,
	ProcessingJobListing,
	ProcessingJobPage,
} from "$lib/types";

/** Rows the table shows before "N more". */
export const JOBS_SHOWN = 25;
// ponytail: queued and retrying share the wire status, so those two filters
// fetch a wider page and split it here. Past 100 waiting jobs the split can
// under-fill; add a `retrying` filter to `list_processing_jobs_by_processor` then.
const SPLIT_FETCH = 100;

export function createDetailStore() {
	let processor = $state<ProcessorKey | null>(null);
	let filter = $state<JobFilter>("all");
	let jobs = $state<ProcessingJobListing[]>([]);
	/** Rows behind the server-side status filter. */
	let total = $state(0);
	/** The inspected job, kept even if it scrolls off the polled page. */
	let inspected = $state<ProcessingJobListing | null>(null);
	let error = $state<string | null>(null);
	let acting = $state(false);
	let actionMessage = $state<string | null>(null);
	/** Ticks with the poll so "Next try in 42s" counts down. */
	let now = $state(Date.now());

	function reset(next: ProcessorKey | null) {
		processor = next;
		filter = "all";
		jobs = [];
		total = 0;
		inspected = null;
		error = null;
		actionMessage = null;
	}

	async function fetchJobs() {
		const key = processor;
		const lane = key ? DETAIL_SPECS[key].processor : null;
		if (!key || !lane) return;
		const requested = filter;
		const split = requested === "queued" || requested === "retrying";
		const request: ListProcessingJobsRequest = {
			processor: lane,
			status: requested === "all" ? null : split ? "queued" : requested,
			subjectId: null,
			limit: split ? SPLIT_FETCH : JOBS_SHOWN,
			offset: 0,
		};
		try {
			const page = await invoke<ProcessingJobPage>("list_processing_jobs_by_processor", { request });
			// Drilled elsewhere or refiltered while in flight: that request owns the view.
			if (processor !== key || filter !== requested) return;
			now = Date.now();
			jobs = page.jobs;
			total = page.total;
			const open = inspected && page.jobs.find((job) => job.id === inspected!.id);
			if (open) inspected = open;
			error = null;
		} catch (err) {
			error = humanizeError(err);
		}
	}

	/** This filter's rows, split client-side for queued/retrying. */
	const visible = $derived(jobs.filter((job) => matchesFilter(jobState(job, now), filter)));

	async function act(run: () => Promise<string>) {
		if (acting) return;
		acting = true;
		actionMessage = null;
		try {
			actionMessage = await run();
			await fetchJobs();
		} catch (err) {
			actionMessage = humanizeError(err);
		} finally {
			acting = false;
		}
	}

	/** Requeue the job's subject through the processor's existing reprocess command. */
	function reprocess(job: ProcessingJobListing) {
		const spec = processor ? DETAIL_SPECS[processor] : null;
		// A subject type the command doesn't take would requeue the wrong row's id.
		if (!spec?.reprocess || job.subjectType !== spec.subjectType) return;
		const { command, arg } = spec.reprocess;
		return act(async () => {
			await invoke(command, { request: { [arg]: job.subjectId } });
			return "Queued again";
		});
	}

	/** Skip a waiting job's backoff (B4). Only valid on a retrying job. */
	function retryNow(jobId: number) {
		return act(async () => {
			const job = await invoke<ProcessingJobDto>("retry_processing_job_now", { jobId });
			if (inspected?.id === job.id) inspected = { ...inspected, ...job, nextAttemptAt: null };
			return "Will try again now";
		});
	}

	return {
		get processor() { return processor; },
		get filter() { return filter; },
		set filter(next: JobFilter) {
			if (next === filter) return;
			filter = next;
			jobs = [];
			total = 0;
			void fetchJobs();
		},
		get jobs() { return jobs; },
		get visible() { return visible; },
		get total() { return total; },
		get inspected() { return inspected; },
		set inspected(next: ProcessingJobListing | null) {
			inspected = next;
			actionMessage = null;
		},
		get error() { return error; },
		get now() { return now; },
		get acting() { return acting; },
		get actionMessage() { return actionMessage; },

		reset,
		fetchJobs,
		reprocess,
		retryNow,
	};
}

export type DetailStore = ReturnType<typeof createDetailStore>;
