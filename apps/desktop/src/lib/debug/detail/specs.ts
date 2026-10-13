// The four processor drill-ins (Pipeline › <processor>). The only
// per-processor code in the drill-in level lives here.
//
// Keyed by `ProcessorKey`, the developer window's own vocabulary — deliberately
// not `DebugFeature`, so the backend can re-cut the health rows without
// touching this table.

import type {
	AudioTranscriptionSettings,
	OcrSettings,
	RecordingSettings,
	SemanticSearchSettings,
	SpeakerAnalysisSettings,
} from "$lib/types";

/** The processors with a drill-in. */
export type ProcessorKey = "reading" | "transcription" | "speakers" | "meaningIndex";

export const PROCESSOR_KEYS: ProcessorKey[] = ["reading", "transcription", "speakers", "meaningIndex"];

/** The settings slice a feature runs on. All four carry `provider` + `modelId`. */
export type FeatureSettings =
	| OcrSettings
	| AudioTranscriptionSettings
	| SpeakerAnalysisSettings
	| SemanticSearchSettings;

export type DetailSpec = {
	id: ProcessorKey;
	/**
	 * `processing_jobs.processor`, or `null` for a feature with **no job lane**.
	 * The meaning index is that case: the semantic index is swept by a worker, not
	 * queued as processing jobs — so it has no jobs table and no per-job actions,
	 * and its hero reads `get_semantic_index_status` instead of a lane.
	 */
	processor: string | null;
	/** The `subject_type` this processor's jobs carry; guards the reprocess call. */
	subjectType: string | null;
	/**
	 * Per-item requeue. These are the reprocess commands that **already exist**;
	 * there is no bulk "reprocess all failed" command, so the detail view offers
	 * none (slice 6 made the same call on the summary card).
	 */
	reprocess: { command: string; /** The request field the subject id goes in. */ arg: string } | null;
	config: (settings: RecordingSettings | null) => FeatureSettings | null;
};

export const DETAIL_SPECS: Record<ProcessorKey, DetailSpec> = {
	reading: {
		id: "reading",
		processor: "ocr",
		subjectType: "frame",
		reprocess: { command: "reprocess_captured_frame_ocr", arg: "frameId" },
		config: (settings) => settings?.ocr ?? null,
	},
	transcription: {
		id: "transcription",
		processor: "audio_transcription",
		subjectType: "audio_segment",
		reprocess: { command: "reprocess_audio_segment_transcription", arg: "audioSegmentId" },
		config: (settings) => settings?.transcription ?? null,
	},
	speakers: {
		id: "speakers",
		processor: "speaker_analysis",
		subjectType: "audio_segment",
		reprocess: { command: "reprocess_audio_segment_speaker_analysis", arg: "audioSegmentId" },
		config: (settings) => settings?.speakerAnalysis ?? null,
	},
	meaningIndex: {
		id: "meaningIndex",
		processor: null,
		subjectType: null,
		reprocess: null,
		config: (settings) => settings?.semanticSearch ?? null,
	},
};
