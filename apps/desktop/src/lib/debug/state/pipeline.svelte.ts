// Pipeline-side debug state: the OCR budget snapshot, app-infra status and the
// background-job list/detail. Fetchers only — the developer window drives them
// through `controller.poll`.

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import type { AppInfraStatus, AppJobDto, OcrBudgetDebug } from "$lib/types";

export function createPipelineStore() {
	// ─── OCR budget ──────────────────────────────────────────────────────────
	let ocrBudgetDebug = $state<OcrBudgetDebug | null>(null);
	let ocrBudgetDebugError = $state<string | null>(null);
	// True while a budget fetch is in flight. Drives first-load skeleton rows so
	// the table can distinguish "fetching" from "never loaded" (the 1s poll keeps
	// toggling this, but the skeleton only renders while no data exists yet).
	let ocrBudgetFetching = $state(false);
	let loadingOcrBudget = $state(false);

	// ─── App infra + jobs ────────────────────────────────────────────────────
	let infraStatus = $state<AppInfraStatus | null>(null);
	let infraStatusError = $state<string | null>(null);
	let loadingInfraStatus = $state(false);

	let jobs = $state<AppJobDto[]>([]);
	let jobsError = $state<string | null>(null);
	let loadingJobs = $state(false);

	let selectedJobId = $state<number | null>(null);
	let selectedJob = $state<AppJobDto | null>(null);
	let loadingSelectedJob = $state(false);
	let selectedJobError = $state<string | null>(null);

	async function fetchOcrBudgetDebug() {
		if (typeof document !== "undefined" && document.visibilityState !== "visible") return;
		ocrBudgetFetching = true;
		try {
			ocrBudgetDebug = await invoke<OcrBudgetDebug>("get_ocr_budget_debug");
			ocrBudgetDebugError = null;
		} catch (err) {
			ocrBudgetDebugError = humanizeError(err);
		} finally {
			ocrBudgetFetching = false;
		}
	}

	async function refreshOcrBudget() {
		loadingOcrBudget = true;
		try {
			await fetchOcrBudgetDebug();
		} finally {
			loadingOcrBudget = false;
		}
	}

	async function fetchInfraStatus() {
		loadingInfraStatus = true;
		infraStatusError = null;
		try {
			infraStatus = await invoke<AppInfraStatus>("get_app_infra_status");
		} catch (err) {
			infraStatusError = humanizeError(err);
		} finally {
			loadingInfraStatus = false;
		}
	}

	async function fetchJobs() {
		loadingJobs = true;
		jobsError = null;
		try {
			jobs = await invoke<AppJobDto[]>("list_app_jobs");
			// Keep selected job detail in sync with the refreshed list. If the
			// selected job is now present in the list, update its detail snapshot so
			// status/result are coherent without a separate round-trip.
			if (selectedJobId != null) {
				const match = jobs.find((j) => j.id === selectedJobId);
				if (match) {
					selectedJob = match;
				} else {
					// Job not found in list — clear stale detail explicitly.
					selectedJob = null;
					selectedJobId = null;
				}
			}
		} catch (err) {
			jobsError = humanizeError(err);
		} finally {
			loadingJobs = false;
		}
	}

	/** Refresh both infra counts and job list together so they stay in sync. */
	async function refreshAll() {
		await Promise.all([fetchInfraStatus(), fetchJobs()]);
	}

	async function selectJob(job: AppJobDto) {
		selectedJobId = job.id;
		selectedJobError = null;
		loadingSelectedJob = true;
		try {
			const result = await invoke<AppJobDto | null>("get_app_job", { request: { jobId: job.id } });
			if (result != null) {
				selectedJob = result;
			} else {
				// Backend says the job no longer exists — clear selection explicitly.
				selectedJob = null;
				selectedJobId = null;
			}
		} catch (err) {
			selectedJobError = humanizeError(err);
			selectedJob = job;
		} finally {
			loadingSelectedJob = false;
		}
	}

	async function refreshSelectedJob() {
		if (selectedJobId == null) return;
		selectedJobError = null;
		loadingSelectedJob = true;
		try {
			const result = await invoke<AppJobDto | null>("get_app_job", { request: { jobId: selectedJobId } });
			if (result != null) {
				selectedJob = result;
			} else {
				// Job no longer found — clear stale detail explicitly.
				selectedJob = null;
				selectedJobId = null;
			}
		} catch (err) {
			selectedJobError = humanizeError(err);
		} finally {
			loadingSelectedJob = false;
		}
	}

	return {
		get ocrBudgetDebug() { return ocrBudgetDebug; },
		get ocrBudgetDebugError() { return ocrBudgetDebugError; },
		get ocrBudgetFetching() { return ocrBudgetFetching; },
		get loadingOcrBudget() { return loadingOcrBudget; },

		get infraStatus() { return infraStatus; },
		get infraStatusError() { return infraStatusError; },
		get loadingInfraStatus() { return loadingInfraStatus; },

		get jobs() { return jobs; },
		get jobsError() { return jobsError; },
		get loadingJobs() { return loadingJobs; },

		get selectedJobId() { return selectedJobId; },
		get selectedJob() { return selectedJob; },
		get loadingSelectedJob() { return loadingSelectedJob; },
		get selectedJobError() { return selectedJobError; },

		fetchOcrBudgetDebug,
		refreshOcrBudget,
		fetchInfraStatus,
		fetchJobs,
		refreshAll,
		selectJob,
		refreshSelectedJob,
	};
}

export type PipelineStore = ReturnType<typeof createPipelineStore>;
