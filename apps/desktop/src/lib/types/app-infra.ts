export type BackgroundJobStatus = "queued" | "running" | "completed" | "failed";

export interface JobCounts {
	total: number;
	queued: number;
	running: number;
	completed: number;
	failed: number;
}

export interface AppInfraStatus {
	databasePath: string;
	migrationsRan: boolean;
	workerThreadCount: number;
	jobCounts: JobCounts;
}

export interface FrameDto {
	id: number;
	sessionId: string;
	filePath: string;
	capturedAt: string;
	width: number | null;
	height: number | null;
	ocrText: string | null;
	processorVersion: string | null;
	equivalenceHint: string | null;
	createdAt: string;
	updatedAt: string;
}

export type FramePreviewSourceKind =
	| "original_frame"
	| "segment_frame_fallback"
	| "video_fallback";

export interface FramePreviewDto {
	mimeType: string;
	filePath: string;
	sourceKind: FramePreviewSourceKind;
}

export interface GetFramePreviewRequest {
	frameId: number;
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

export interface ListAudioSegmentsRequest {
	capturedAtStart: string;
	capturedAtEnd: string;
}

export interface GetAudioSegmentMediaRequest {
	audioSegmentId: number;
}

export interface AudioSegmentMediaDto {
	mimeType: string;
	dataBase64: string;
}

export type FrameBatchStatus = "open" | "closed" | "processing" | "completed" | "failed";
export type ProcessingJobStatus = "queued" | "running" | "completed" | "failed";

export type SegmentWorkspaceCleanupDisposition =
	| "referenced_by_incomplete_batch"
	| "referenced_by_nonterminal_ocr"
	| "missing_visible_segment_sibling"
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
	payloadJson: string | null;
	lastError: string | null;
	createdAt: string;
	updatedAt: string;
	startedAt: string | null;
	finishedAt: string | null;
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

export interface OcrStructuredPayload {
	schemaVersion: number;
	coordinateSpace: "normalized" | string;
	coordinateOrigin: "lower_left" | string;
	observations: OcrObservation[];
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
