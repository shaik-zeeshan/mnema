// Pipeline-side debug state: the OCR budget snapshot (+ its two event tables),
// app-infra status, the background-job list/detail/submit flow, and the hidden
// segment-workspace classifier.
//
// A behaviour-preserving move of the pipeline half of the legacy
// routes/debug/+page.svelte <script>.

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import type {
	AppInfraStatus,
	AppJobDto,
	OcrBudgetDebug,
	SegmentWorkspaceCleanupDebugInfoDto,
} from "$lib/types";

const OCR_EVENT_PAGE_SIZE = 10;
const JOBS_PAGE_SIZE = 5;
const POST_SUBMIT_POLL_MAX = 8; // poll up to ~8s after submit then stop
const POST_SUBMIT_POLL_MS = 1000;
const OCR_BUDGET_POLL_MS = 1000;

export function createPipelineStore() {
	// ─── OCR budget ──────────────────────────────────────────────────────────
	let ocrBudgetDebug = $state<OcrBudgetDebug | null>(null);
	let ocrBudgetDebugError = $state<string | null>(null);
	// True while a budget fetch is in flight. Drives first-load skeleton rows so
	// the table can distinguish "fetching" from "never loaded" (the 1s poll keeps
	// toggling this, but the skeleton only renders while no data exists yet).
	let ocrBudgetFetching = $state(false);
	let loadingOcrBudget = $state(false);
	let admissionPage = $state(0);
	let executionPage = $state(0);

	// ─── App infra + jobs ────────────────────────────────────────────────────
	let infraStatus = $state<AppInfraStatus | null>(null);
	let infraStatusError = $state<string | null>(null);
	let loadingInfraStatus = $state(false);

	let jobs = $state<AppJobDto[]>([]);
	let jobsError = $state<string | null>(null);
	let loadingJobs = $state(false);
	let jobsPage = $state(0);

	let selectedJobId = $state<number | null>(null);
	let selectedJob = $state<AppJobDto | null>(null);
	let loadingSelectedJob = $state(false);
	let selectedJobError = $state<string | null>(null);

	let submitDocName = $state("");
	let submitSourceText = $state("");
	let submitting = $state(false);
	let submitError = $state<string | null>(null);

	// Tracks the active post-submit polling interval so it can be cancelled.
	let postSubmitPollInterval = $state<ReturnType<typeof setInterval> | null>(null);
	let postSubmitPollCount = $state(0);

	// ─── Hidden segment workspace classifier ─────────────────────────────────
	let workspaceDirInput = $state("");
	let workspaceClassification = $state<SegmentWorkspaceCleanupDebugInfoDto | null>(null);
	// `null` here means "no path looked like a hidden segment workspace" (the
	// backend returned `Option::None`); distinct from "have not run yet".
	let workspaceClassificationLoaded = $state(false);
	let workspaceClassificationError = $state<string | null>(null);
	let loadingWorkspaceClassification = $state(false);

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

	/** 1s OCR-budget poll, paused while the document is hidden. Returns teardown. */
	function startOcrBudgetPolling(): () => void {
		void fetchOcrBudgetDebug();
		const interval = setInterval(() => { void fetchOcrBudgetDebug(); }, OCR_BUDGET_POLL_MS);
		const onVisibility = () => {
			if (document.visibilityState === "visible") void fetchOcrBudgetDebug();
		};
		document.addEventListener("visibilitychange", onVisibility);
		return () => {
			clearInterval(interval);
			document.removeEventListener("visibilitychange", onVisibility);
		};
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

	function stopPostSubmitPolling() {
		if (postSubmitPollInterval != null) {
			clearInterval(postSubmitPollInterval);
			postSubmitPollInterval = null;
			postSubmitPollCount = 0;
		}
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

	async function submitDebugJob() {
		submitting = true;
		submitError = null;
		// Cancel any existing post-submit poll before starting a new one.
		stopPostSubmitPolling();
		try {
			const newJob = await invoke<AppJobDto>("submit_debug_cpu_job", {
				request: { documentName: submitDocName, sourceText: submitSourceText },
			});
			jobs = [newJob, ...jobs];
			submitDocName = "";
			submitSourceText = "";
			// Start a short-lived polling window to catch status updates quickly.
			postSubmitPollCount = 0;
			postSubmitPollInterval = setInterval(async () => {
				postSubmitPollCount += 1;
				await refreshAll();
				if (postSubmitPollCount >= POST_SUBMIT_POLL_MAX) stopPostSubmitPolling();
			}, POST_SUBMIT_POLL_MS);
		} catch (err) {
			submitError = humanizeError(err);
		} finally {
			submitting = false;
		}
	}

	async function classifyWorkspace() {
		const trimmed = workspaceDirInput.trim();
		if (!trimmed) {
			workspaceClassificationError = "workspace path is required";
			return;
		}
		loadingWorkspaceClassification = true;
		workspaceClassificationError = null;
		try {
			workspaceClassification = await invoke<SegmentWorkspaceCleanupDebugInfoDto | null>(
				"classify_hidden_segment_workspace",
				{ request: { workspaceDir: trimmed } }
			);
			workspaceClassificationLoaded = true;
		} catch (err) {
			workspaceClassification = null;
			workspaceClassificationLoaded = false;
			workspaceClassificationError = humanizeError(err);
		} finally {
			loadingWorkspaceClassification = false;
		}
	}

	// ─── Pagination ──────────────────────────────────────────────────────────
	// Recent-jobs can grow unbounded; render a fixed-size window. The selected
	// job detail panel is rendered outside the paginated list so the user can
	// still see it after paging away from the row that owns it.
	const jobsPageCount = $derived(Math.max(1, Math.ceil(jobs.length / JOBS_PAGE_SIZE)));
	const admissionPageCount = $derived(
		Math.max(1, Math.ceil((ocrBudgetDebug?.admissionEvents.length ?? 0) / OCR_EVENT_PAGE_SIZE))
	);
	const executionPageCount = $derived(
		Math.max(1, Math.ceil((ocrBudgetDebug?.executionEvents.length ?? 0) / OCR_EVENT_PAGE_SIZE))
	);
	const jobsPageStart = $derived(jobsPage * JOBS_PAGE_SIZE);
	const selectedJobPage = $derived.by(() => {
		if (selectedJobId == null) return null;
		const idx = jobs.findIndex((j) => j.id === selectedJobId);
		if (idx < 0) return null;
		return Math.floor(idx / JOBS_PAGE_SIZE);
	});

	/**
	 * Clamp every pager when its underlying list shrinks (e.g. after a refresh
	 * that drops a previously-listed job) — otherwise the user lands on an empty
	 * page that confusingly shows no rows despite the list being non-empty.
	 * Driven by one $effect in the owning section.
	 */
	function clampPages() {
		if (jobsPage > jobsPageCount - 1) jobsPage = jobsPageCount - 1;
		if (jobsPage < 0) jobsPage = 0;
		if (admissionPage > admissionPageCount - 1) admissionPage = admissionPageCount - 1;
		if (admissionPage < 0) admissionPage = 0;
		if (executionPage > executionPageCount - 1) executionPage = executionPageCount - 1;
		if (executionPage < 0) executionPage = 0;
	}

	return {
		get ocrBudgetDebug() { return ocrBudgetDebug; },
		get ocrBudgetDebugError() { return ocrBudgetDebugError; },
		get ocrBudgetFetching() { return ocrBudgetFetching; },
		get loadingOcrBudget() { return loadingOcrBudget; },
		get admissionPage() { return admissionPage; },
		set admissionPage(v: number) { admissionPage = v; },
		get executionPage() { return executionPage; },
		set executionPage(v: number) { executionPage = v; },
		get admissionPageCount() { return admissionPageCount; },
		get executionPageCount() { return executionPageCount; },
		get pagedAdmissionEvents() {
			return (ocrBudgetDebug?.admissionEvents ?? [])
				.slice(admissionPage * OCR_EVENT_PAGE_SIZE, admissionPage * OCR_EVENT_PAGE_SIZE + OCR_EVENT_PAGE_SIZE);
		},
		get pagedExecutionEvents() {
			return (ocrBudgetDebug?.executionEvents ?? [])
				.slice(executionPage * OCR_EVENT_PAGE_SIZE, executionPage * OCR_EVENT_PAGE_SIZE + OCR_EVENT_PAGE_SIZE);
		},

		get infraStatus() { return infraStatus; },
		get infraStatusError() { return infraStatusError; },
		get loadingInfraStatus() { return loadingInfraStatus; },

		get jobs() { return jobs; },
		get jobsError() { return jobsError; },
		get loadingJobs() { return loadingJobs; },
		get jobsPage() { return jobsPage; },
		set jobsPage(v: number) { jobsPage = v; },
		get jobsPageCount() { return jobsPageCount; },
		get jobsPageStart() { return jobsPageStart; },
		get jobsPageSize() { return JOBS_PAGE_SIZE; },
		get pagedJobs() { return jobs.slice(jobsPageStart, jobsPageStart + JOBS_PAGE_SIZE); },
		get selectedJobOnAnotherPage() { return selectedJobPage != null && selectedJobPage !== jobsPage; },
		goToSelectedJobPage() { if (selectedJobPage != null) jobsPage = selectedJobPage; },

		get selectedJobId() { return selectedJobId; },
		get selectedJob() { return selectedJob; },
		get loadingSelectedJob() { return loadingSelectedJob; },
		get selectedJobError() { return selectedJobError; },

		get submitDocName() { return submitDocName; },
		set submitDocName(v: string) { submitDocName = v; },
		get submitSourceText() { return submitSourceText; },
		set submitSourceText(v: string) { submitSourceText = v; },
		get submitting() { return submitting; },
		get submitError() { return submitError; },
		get postSubmitPolling() { return postSubmitPollInterval != null; },
		get postSubmitPollsLeft() { return POST_SUBMIT_POLL_MAX - postSubmitPollCount; },

		get workspaceDirInput() { return workspaceDirInput; },
		set workspaceDirInput(v: string) { workspaceDirInput = v; },
		get workspaceClassification() { return workspaceClassification; },
		get workspaceClassificationLoaded() { return workspaceClassificationLoaded; },
		get workspaceClassificationError() { return workspaceClassificationError; },
		get loadingWorkspaceClassification() { return loadingWorkspaceClassification; },

		fetchOcrBudgetDebug,
		refreshOcrBudget,
		startOcrBudgetPolling,
		fetchInfraStatus,
		fetchJobs,
		refreshAll,
		stopPostSubmitPolling,
		selectJob,
		refreshSelectedJob,
		submitDebugJob,
		classifyWorkspace,
		clampPages,
	};
}

export type PipelineStore = ReturnType<typeof createPipelineStore>;
