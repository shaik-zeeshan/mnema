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

export interface FrameMetadataSnapshot {
	appBundleId: string | null;
	appName: string | null;
	windowTitle: string | null;
	browserUrl: string | null;
	displayId: number | null;
	metadataRedactionReason: string | null;
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
