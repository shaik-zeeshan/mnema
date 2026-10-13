export type BackgroundJobStatus = "queued" | "running" | "completed" | "failed";

export interface JobCounts {
	total: number;
	queued: number;
	running: number;
	completed: number;
	failed: number;
}

export interface FrameBatchCounts {
	open: number;
	failed: number;
}

export interface AppInfraStatus {
	databasePath: string;
	/** Size of the main DB file in bytes; null when it cannot be stat'd. */
	databaseSizeBytes: number | null;
	migrationsRan: boolean;
	/** Rows in `_sqlx_migrations`; null when the count query fails. */
	appliedMigrationCount: number | null;
	workerThreadCount: number;
	jobCounts: JobCounts;
	frameBatchCounts: FrameBatchCounts;
}

export interface FrameMetadataSnapshot {
	appBundleId: string | null;
	appName: string | null;
	windowTitle: string | null;
	browserUrl: string | null;
	displayId: number | null;
	metadataRedactionReason: string | null;
	metadataRedactionSourceId: string | null;
}

export interface FrameDto {
	id: number;
	sessionId: string;
	filePath: string;
	capturedAt: string;
	width: number | null;
	height: number | null;
	appBundleId: string | null;
	appName: string | null;
	windowTitle: string | null;
	url: string | null;
	ocrText: string | null;
	processorVersion: string | null;
	equivalenceHint: string | null;
	createdAt: string;
	updatedAt: string;
}

export type FramePreviewSourceKind =
	| "original_frame"
	| "segment_frame_fallback"
	| "video_fallback"
	| "scrub_preview";

export interface FramePreviewDto {
	mimeType: string;
	filePath: string;
	sourceKind: FramePreviewSourceKind;
	hasSecretRedactions: boolean;
	secretRedactionCount: number;
}

export type FramePreviewVideoScope = "active_frame";

export interface GetFramePreviewRequest {
	frameId: number;
	videoScope?: FramePreviewVideoScope;
}

export type ScrubPreviewMissingReason =
	| "frame_not_found"
	| "direct_file_missing"
	| "segment_unresolved"
	| "frame_index_missing"
	| "frame_index_entry_missing"
	| "segment_video_missing"
	| "decode_failed"
	| "cache_write_failed";

export interface GetFrameScrubPreviewsRequest {
	frameIds: number[];
	maxPixelSize?: number;
}

export interface FrameScrubPreviewResultDto {
	frameId: number;
	preview: FramePreviewDto | null;
	missingReason: ScrubPreviewMissingReason | null;
}

export interface FrameScrubPreviewsDto {
	previews: FrameScrubPreviewResultDto[];
}

export type ScrubPreviewAvailabilityStatus =
	| "ready"
	| "queued"
	| "not_indexed"
	| "source_missing"
	| "frame_index_missing"
	| "decode_failed"
	| "cache_write_failed"
	| "unsupported_platform";

export interface GetScrubPreviewAvailabilityRequest {
	startUnixMs: number;
	endUnixMs: number;
	enqueueMissing?: boolean;
}

export interface ScrubPreviewAvailabilityIntervalDto {
	segmentCacheKey: string;
	intervalStartVideoOffsetMs: number;
	intervalEndVideoOffsetMs: number;
	intervalStartUnixMs: number;
	intervalEndUnixMs: number;
	preview: FramePreviewDto | null;
	status: ScrubPreviewAvailabilityStatus;
}

export interface ScrubPreviewAvailabilityDto {
	rendition: "v1-jpeg-q72-max360-1fps";
	intervals: ScrubPreviewAvailabilityIntervalDto[];
}

export interface GetTimelineWindowAroundFrameRequest {
	frameId: number;
	newerLimit: number;
	olderLimit: number;
}

export interface GetNearestEarlierEquivalentFrameRequest {
	frameId: number;
}

export interface GetEarliestEarlierEquivalentFrameRequest {
	frameId: number;
}

export interface FocusedFrameWindowDto {
	frames: FrameDto[];
	targetIndex: number;
	hasNewer: boolean;
	hasOlder: boolean;
}

export interface FrameSummaryDto {
	id: number;
	capturedAt: string;
}

export interface FrameRangeRequest {
	capturedAtStart: string;
	capturedAtEnd: string;
}

export interface ListFramesRequest {
	sessionId?: string | null;
	limit?: number | null;
	offset?: number | null;
	beforeId?: number | null;
}

export type AudioSegmentSourceKind = "microphone" | "system_audio";

export interface AudioSegmentDto {
	id: number;
	sourceKind: AudioSegmentSourceKind;
	sourceSessionId: string;
	segmentIndex: number;
	filePath: string;
	startedAt: string;
	endedAt: string;
	createdAt: string;
	updatedAt: string;
}

export interface SearchCaptureRequest {
	query: string;
	frameLimit?: number;
	frameOffset?: number;
	audioLimit?: number;
	audioOffset?: number;
	snapshotDocumentId?: number;
	refinements?: SearchCaptureRefinements;
}

export interface SearchCaptureRefinements {
	dateRange?: SearchDateRangeRefinement;
	apps?: SearchAppRefinement[];
	windowTitle?: string;
	audioSources?: AudioSegmentSourceKind[];
	// `source:screen` — restricts results to captured frames (screen), the
	// frame-side counterpart of audioSources. Mutually exclusive with them.
	screenSource?: boolean;
	/** `hours:H-H` — local time-of-day window; see {@link SearchHoursRefinement}. */
	hours?: SearchHoursRefinement;
}

/**
 * Local time of day, `startHour <= hour < endHour` (0–24). Wraps past midnight
 * when `startHour > endHour` (`hours:22-6`). A hit is placed by its start.
 */
export interface SearchHoursRefinement {
	startHour: number;
	endHour: number;
}

/**
 * One Quick Recall recent query (`get_quick_recall_recent_queries` /
 * `set_quick_recall_recent_queries`, newest first, at most 8). `at` = unix ms.
 * Kept in the encrypted DB; pruned by Retention, cleared by Wipe User Context,
 * untouched by Delete Recent Capture.
 */
export interface RecentQuery {
	text: string;
	ask: boolean;
	at: number;
}

export interface SearchDateRangeRefinement {
	startAt: string;
	endAt: string;
	origin?: "visible_timeline" | "today" | "last_hour";
}

export interface SearchAppRefinement {
	kind: "any" | "bundle_id" | "app_name";
	value: string;
	displayName: string;
}

export interface SearchParseError {
	kind: string;
	message: string;
	start: number;
	end: number;
	token: string;
}

export interface SearchableApp {
	bundleId: string | null;
	name: string | null;
}

export interface SearchCaptureResponse {
	normalizedQuery: string;
	snapshotDocumentId: number;
	frames: FrameSearchResultDto[];
	audio: AudioSearchResultDto[];
	hasMoreFrames: boolean;
	hasMoreAudio: boolean;
	appliedRefinements: SearchCaptureRefinements;
	residualQuery: string;
	parseErrors: SearchParseError[];
	/** Nothing is indexed at all — "nothing captured yet", not "no match". */
	indexEmpty: boolean;
	/** The meaning tier failed; this page is keyword-only. */
	semanticDegraded: boolean;
	/**
	 * Audio segments in the searched range still queued or being transcribed
	 * (audio is searchable only once transcribed). 0 when the audio pass was
	 * skipped by an app / window-title / `source:screen` filter.
	 */
	untranscribedSegmentsInRange: number;
}

/**
 * `count_search_facet` — which pill's refinement to swap out. Buckets: `app`
 * → bundle id (or name) with the display name as `label`, frames only; `source`
 * → "screen" | "microphone" | "system_audio"; `day` → local "YYYY-MM-DD";
 * `partOfDay` → "morning" 5–12 | "afternoon" 12–18 | "evening" 18–24 | "night" 0–5.
 */
export type SearchFacet = "app" | "source" | "day" | "partOfDay";

export interface CountSearchFacetRequest {
	/** The same query `search_capture` ran, operators included. */
	query: string;
	refinements?: SearchCaptureRefinements;
	facet: SearchFacet;
}

export interface SearchFacetBucket {
	key: string;
	label: string;
	count: number;
}

export interface SearchFacetCounts {
	facet: SearchFacet;
	/** Every result with the facet's refinement removed ("Any app 9"). */
	total: number;
	/** A kind hit the 50-result cap: counts are lower bounds ("50+"). */
	capped: boolean;
	/** Most results first. */
	buckets: SearchFacetBucket[];
}

/** `get_quick_recall_index_status` — what the partial / no-results states say. */
export interface QuickRecallIndexStatus {
	/** Frames captured but not yet read (OCR queued or running). */
	queuedFrames: number;
	/** Capture time of the oldest unread frame, unix ms. */
	oldestQueuedMs: number | null;
	/** Meaning search is on but still embedding history. */
	semanticBuilding: boolean;
	/** 0–100; null when meaning search is off (no model) = "keyword only". */
	semanticIndexedPercent: number | null;
}

/**
 * One matched OCR line on a frame, normalised 0–1 with a TOP-LEFT origin.
 * `keyword` = contains a query word (outline solid); `meaning` = a meaning-only
 * hit's word-stem match (dashed). Empty list = no geometry or no match: show the
 * whole frame.
 */
export interface MatchBox {
	x: number;
	y: number;
	w: number;
	h: number;
	kind: "keyword" | "meaning";
}

/** `get_frame_match_boxes` — boxes for any frame (Quick Look neighbours). */
export interface GetFrameMatchBoxesRequest {
	frameId: number;
	query: string;
	byMeaning?: boolean;
}

export interface FrameSearchResultDto {
	/**
	 * Relevance, LOWER is better. Comparable with {@link AudioSearchResultDto.rank}
	 * for a two-pointer merge (tie → newer, then frame). Never calibrated (ADR 0036).
	 */
	rank: number;
	groupKey: string;
	representativeFrame: FrameDto;
	groupStartAt: string;
	groupEndAt: string;
	matchCount: number;
	snippet: string;
	appBundleId: string | null;
	appName: string | null;
	windowTitle: string | null;
	/**
	 * The read-time-guarded host+path of the captured page (e.g.
	 * "github.com/owner/repo/commit/9fceb…"), or null when there is no openable
	 * http(s) URL. The raw URL stays in Rust; only this guarded form is ever
	 * shown, and the page is opened via `open_captured_url` with
	 * `thumbnailFrameId`.
	 */
	url: string | null;
	thumbnailFrameId: number;
	textSourceKind: "direct" | "equivalent_reuse";
	hasSecretRedactions: boolean;
	secretRedactionCount: number;
	/**
	 * A meaning-only Semantic Search hit: the snippet is a leading body-text
	 * excerpt that should render with a "found by meaning" tag rather than a
	 * highlighted keyword snippet.
	 */
	foundByMeaning: boolean;
	/** Where the query's words sit on `representativeFrame`, at most 6. */
	matchBoxes: MatchBox[];
}

export interface AudioSearchResultDto {
	/** Relevance, LOWER is better (see {@link FrameSearchResultDto.rank}). */
	rank: number;
	groupKey: string;
	audioSegment: AudioSegmentDto;
	sourceKind: AudioSegmentSourceKind;
	spanStartMs: number;
	spanEndMs: number;
	absoluteStartAt: string;
	absoluteEndAt: string;
	matchCount: number;
	snippet: string;
	alignedFrame: FrameDto | null;
	hasSecretRedactions: boolean;
	secretRedactionCount: number;
	/** A meaning-only Semantic Search hit (see {@link FrameSearchResultDto}). */
	foundByMeaning: boolean;
}

export interface ListAudioSegmentsRequest {
	capturedAtStart: string;
	capturedAtEnd: string;
}

export interface GetAudioSegmentMediaRequest {
	audioSegmentId: number;
}

export interface GetAudioSegmentRequest {
	audioSegmentId: number;
}

/** Amplitude peaks for the transcription reader's waveform scrubber. The command
 *  returns an EMPTY array on any failure — that means "plain scrub bar", not an
 *  error state. */
export interface GetAudioSegmentWaveformPeaksRequest {
	audioSegmentId: number;
	bucketCount: number;
}

export interface AudioSegmentMediaDto {
	mimeType: string;
	dataBase64: string;
}

export interface ListSpeakerTurnsRequest {
	audioSegmentId: number;
}

export interface ListSpeakerClustersRequest {
	sessionId: string;
}

export interface CreatePersonProfileRequest {
	displayName: string;
	notes?: string | null;
}

export interface DeletePersonProfileRequest {
	personId: number;
}

export interface NameSpeakerClusterRequest {
	clusterId: number;
	label: string;
}

export interface LinkSpeakerClusterRequest {
	clusterId: number;
	personId: number;
	addEmbedding: boolean;
}

export interface SpeakerClusterRequest {
	clusterId: number;
}

export interface ConfirmSpeakerSuggestionRequest {
	clusterId: number;
	addEmbedding: boolean;
}

export interface MergeSpeakerClustersRequest {
	sourceClusterId: number;
	targetClusterId: number;
}

export interface MoveSpeakerTurnRequest {
	turnId: number;
	targetClusterId: number;
}

export interface ReprocessAudioSegmentSpeakerAnalysisRequest {
	audioSegmentId: number;
}

export type SpeakerRecognitionConfidence = "high" | "medium" | "low";

export interface SpeakerTurnDto {
	id: number;
	audioSegmentId: number;
	sessionId: string;
	clusterId: number;
	segmentClusterId: number | null;
	providerClusterId: string;
	speakerLabel: string;
	personId: number | null;
	/** `personId` was decided by owner-only auto-linking, not by a human. */
	personLinkAuto: boolean;
	suggestedPersonId: number | null;
	recognitionConfidence: SpeakerRecognitionConfidence | null;
	recognitionScore: number | null;
	startMs: number;
	endMs: number;
	transcriptText: string | null;
	overlaps: boolean;
}

export interface PersonProfileDto {
	id: number;
	displayName: string;
	notes: string | null;
	embeddingCount: number;
	isAccountOwner: boolean;
	createdAt: string;
	updatedAt: string;
}

export interface SpeakerClusterDto {
	id: number;
	sessionId: string;
	provider: string;
	modelId: string | null;
	providerClusterId: string;
	speakerLabel: string;
	personId: number | null;
	/** `personId` was decided by owner-only auto-linking, not by a human. */
	personLinkAuto: boolean;
	suggestedPersonId: number | null;
	recognitionConfidence: SpeakerRecognitionConfidence | null;
	recognitionScore: number | null;
	suggestedMergeTargetClusterId: number | null;
	suggestedMergeScore: number | null;
}

export type FrameBatchStatus = "open" | "closed" | "processing" | "completed" | "failed";
export type ProcessingJobStatus = "queued" | "running" | "completed" | "failed";

export type SegmentWorkspaceCleanupDisposition =
	| "referenced_by_incomplete_batch"
	| "referenced_by_nonterminal_ocr"
	| "missing_visible_segment_sibling"
	| "dead_segment_without_artifacts"
	| "pending_frame_artifacts"
	| "completed_only"
	| "no_references";

export interface HiddenSegmentWorkspacePathsDto {
	workspaceDir: string;
	framesDir: string;
	visibleSegmentPath: string;
}

export interface SegmentWorkspaceBatchReferenceDto {
	batchId: number;
	status: FrameBatchStatus;
}

export interface SegmentWorkspaceOcrReferenceDto {
	frameId: number;
	jobId: number;
	status: ProcessingJobStatus;
}

export interface SegmentWorkspaceCleanupDebugInfoDto {
	paths: HiddenSegmentWorkspacePathsDto;
	disposition: SegmentWorkspaceCleanupDisposition;
	safeToRemove: boolean;
	visibleSegmentExists: boolean;
	frameCount: number;
	batchReferences: SegmentWorkspaceBatchReferenceDto[];
	nonterminalOcrReferences: SegmentWorkspaceOcrReferenceDto[];
}

export interface ProcessingJobDto {
	id: number;
	subjectType: string;
	subjectId: number;
	processor: string;
	status: ProcessingJobStatus;
	attemptCount: number;
	failureCount: number;
	payloadJson: string | null;
	lastError: string | null;
	createdAt: string;
	queuedAt: string;
	updatedAt: string;
	startedAt: string | null;
	finishedAt: string | null;
	/** Queued but its model is downloading/absent ("Preparing"). Only job-listing commands resolve it. */
	modelLocked: boolean;
}

export interface ProcessingResultDto {
	id: number;
	jobId: number;
	subjectType: string;
	subjectId: number;
	processor: string;
	resultText: string | null;
	structuredPayloadJson: string | null;
	processorVersion: string | null;
	createdAt: string;
}

export interface TranscriptionSegment {
	startMs: number;
	endMs: number;
	text: string;
	confidence?: number | null;
}

export interface TranscriptionWord {
	startMs: number;
	endMs: number;
	text: string;
	confidence?: number | null;
}

export interface TranscriptionStructuredPayload {
	provider: string;
	modelId?: string | null;
	language: string;
	segments: TranscriptionSegment[];
	words: TranscriptionWord[];
	provenance?: Record<string, unknown>;
}

export type SpeakerAnalysisSkipReason = "too_short" | "silent";

export interface SpeakerAnalysisProvenance {
	schemaVersion?: number;
	audioDurationMs?: number;
	audioPeak?: number;
	skipReason?: SpeakerAnalysisSkipReason | null;
	chunkingMode?: "single" | "safe_chunked" | string;
	chunkCount?: number;
	turnCount?: number;
	clusterCount?: number;
	recognitionEnabled?: boolean;
	warningReasons?: string[];
	segmentationModelPath?: string;
	embeddingModelPath?: string;
}

export interface SpeakerAnalysisStructuredPayload {
	clusters?: unknown[];
	turns?: unknown[];
	metadata?: {
		provenance?: SpeakerAnalysisProvenance | null;
	} | null;
}

export type CapturedFrameReprocessingOutcome = "created" | "ignored" | "requeued";

export interface CapturedFrameReprocessingResultDto {
	outcome: CapturedFrameReprocessingOutcome;
	job: ProcessingJobDto;
}

export type AudioSegmentTranscriptionReprocessingOutcome = "created" | "ignored" | "requeued";

export interface AudioSegmentTranscriptionReprocessingResultDto {
	outcome: AudioSegmentTranscriptionReprocessingOutcome;
	job: ProcessingJobDto;
}

export interface SystemAudioSpeechActivityReprocessingResultDto {
	outcome: AudioSegmentTranscriptionReprocessingOutcome;
	job: ProcessingJobDto;
}

export interface ReprocessCapturedFrameOcrRequest {
	frameId: number;
	payloadJson?: string | null;
}

export interface ReprocessAudioSegmentTranscriptionRequest {
	audioSegmentId: number;
}

export interface GetProcessingJobRequest {
	jobId: number;
}

export interface GetProcessingResultRequest {
	jobId: number;
}

export interface OcrBoundingBox {
	x: number;
	y: number;
	width: number;
	height: number;
}

export interface OcrObservation {
	text: string;
	confidence: number;
	boundingBox: OcrBoundingBox;
}

export interface OcrProvenance {
	provider: string;
	modelId?: string | null;
}

export interface OcrStructuredPayload {
	schemaVersion: number;
	coordinateSpace: "normalized" | string;
	coordinateOrigin: "lower_left" | string;
	provider?: string;
	modelId?: string | null;
	observations: OcrObservation[];
	provenance?: OcrProvenance | null;
}

export interface AppJobDto {
	id: number;
	kind: string;
	status: BackgroundJobStatus;
	payloadJson: string | null;
	resultText: string | null;
	attemptCount: number;
	lastError: string | null;
	createdAt: string;
	updatedAt: string;
	startedAt: string | null;
	finishedAt: string | null;
}

/** `get_storage_usage` (`managed_storage_layout.rs` ↔ app-infra `StorageUsage`). */
export interface StorageUsage {
	screenBytes: number;
	microphoneBytes: number;
	systemAudioBytes: number;
	indexBytes: number;
	modelsBytes: number;
	/** Distinct recording days on disk — the per-day rate's denominator. */
	captureDays: number;
	/** Null when the volume could not be read. */
	freeBytes: number | null;
}
