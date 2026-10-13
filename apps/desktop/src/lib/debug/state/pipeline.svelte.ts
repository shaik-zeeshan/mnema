// Pipeline-tab state: the processor lanes + meaning index (the 2 s poll) and
// housekeeping (retention cleanup, frame batches, app jobs; on open + 30 s).
// Fetchers only — the tab drives them through `controller.poll`.
//
// Lanes live here rather than in `features.svelte.ts` because that store's
// `poll()` also reads AI-tab data the Pipeline budget doesn't allow.

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import type { AppJobDto, ProcessorPipelineStatus, RecordingSettings, SemanticIndexStatusDto } from "$lib/types";
import { laneFor } from "./features.svelte";
import type { FrameBatch, RetentionCleanupSummary } from "../pipeline-view";

export function createPipelineStore() {
	let lanes = $state<ProcessorPipelineStatus[]>([]);
	let semanticIndex = $state<SemanticIndexStatusDto | null>(null);
	let error = $state<string | null>(null);

	let retention = $state<RetentionCleanupSummary | null>(null);
	let batches = $state<FrameBatch[]>([]);
	let appJobs = $state<AppJobDto[]>([]);
	let housekeepingError = $state<string | null>(null);
	// The processor rows name each engine; settings ride the 30 s housekeeping tick.
	let settings = $state<RecordingSettings | null>(null);

	/** Lanes + meaning index: cheap DB reads, polled at 2 s while the tab is open. */
	async function fetchStatus() {
		try {
			[lanes, semanticIndex] = await Promise.all([
				invoke<ProcessorPipelineStatus[]>("get_processing_pipeline_status"),
				invoke<SemanticIndexStatusDto>("get_semantic_index_status"),
			]);
			error = null;
		} catch (err) {
			error = humanizeError(err);
		}
	}

	async function fetchHousekeeping() {
		invoke<RecordingSettings>("get_recording_settings").then((v) => (settings = v)).catch(() => {});
		try {
			[retention, batches, appJobs] = await Promise.all([
				invoke<RetentionCleanupSummary>("get_retention_cleanup_status"),
				invoke<FrameBatch[]>("list_frame_batches", { limit: 50 }),
				invoke<AppJobDto[]>("list_app_jobs"),
			]);
			housekeepingError = null;
		} catch (err) {
			housekeepingError = humanizeError(err);
		}
	}

	return {
		get lanes() { return lanes; },
		/** A processor's lane, zero-defaulted (a lane with no jobs is absent). */
		lane(processor: string): ProcessorPipelineStatus {
			return laneFor(lanes, processor);
		},
		get semanticIndex() { return semanticIndex; },
		get error() { return error; },

		get retention() { return retention; },
		get batches() { return batches; },
		get appJobs() { return appJobs; },
		get housekeepingError() { return housekeepingError; },
		get settings() { return settings; },

		fetchStatus,
		fetchHousekeeping,
	};
}

export type PipelineStore = ReturnType<typeof createPipelineStore>;
