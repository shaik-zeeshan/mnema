// The four feature-detail instantiations. ONE generic template
// (`FeatureDetail.svelte`) reads this table — the only per-feature code in the
// drill-in level lives here.
//
// A `DetailFeatureId` is deliberately also a `DebugSectionId` *and* a
// `DebugFeature`, so the breadcrumb label, the dock anchor, the health
// dot/diagnosis and the log-tail chip all come from the registries that already
// exist (`sections.ts`, `get_debug_health`) instead of being restated here.

import type {
	AudioTranscriptionSettings,
	OcrSettings,
	RecordingSettings,
	SemanticSearchSettings,
	SpeakerAnalysisSettings,
} from "$lib/types";

/** The features with a drill-in. Each is a `DebugSectionId` and a `DebugFeature`. */
export type DetailFeatureId = "ocr" | "transcription" | "diarization" | "embeddings";

export const DETAIL_FEATURE_IDS: DetailFeatureId[] = ["ocr", "transcription", "diarization", "embeddings"];

/** The settings slice a feature runs on. All four carry `provider` + `modelId`. */
export type FeatureSettings =
	| OcrSettings
	| AudioTranscriptionSettings
	| SpeakerAnalysisSettings
	| SemanticSearchSettings;

export type DetailSpec = {
	id: DetailFeatureId;
	/**
	 * `processing_jobs.processor`, or `null` for a feature with **no job lane**.
	 * Embeddings is that case: the semantic index is swept by a worker, not
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

export const DETAIL_SPECS: Record<DetailFeatureId, DetailSpec> = {
	ocr: {
		id: "ocr",
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
	diarization: {
		id: "diarization",
		processor: "speaker_analysis",
		subjectType: "audio_segment",
		reprocess: { command: "reprocess_audio_segment_speaker_analysis", arg: "audioSegmentId" },
		config: (settings) => settings?.speakerAnalysis ?? null,
	},
	embeddings: {
		id: "embeddings",
		processor: null,
		subjectType: null,
		reprocess: null,
		config: (settings) => settings?.semanticSearch ?? null,
	},
};
