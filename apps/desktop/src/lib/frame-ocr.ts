// Pure/reusable OCR helpers shared by the Timeline and the FrameDetailModal.
//
// These were lifted verbatim out of `routes/+page.svelte` so both surfaces can
// load and render OCR overlays without duplicating the payload-parsing and
// box-geometry logic. Everything here is free of Svelte reactivity; the
// invoke-based loaders take the tauri `invoke` function as a parameter so they
// stay pure and testable (dependency injection — no direct tauri import).
//
// The `$state` machine (poll scheduling, generation tokens, active-frame
// tracking) intentionally stays in the Svelte component.

import type {
  FrameDto,
  GetProcessingResultRequest,
  OcrObservation,
  OcrStructuredPayload,
  ProcessingJobDto,
  ProcessingResultDto,
} from "$lib/types/app-infra";

// Injected tauri invoke. Kept narrow to what the loaders need; matches the
// `@tauri-apps/api/core` `invoke` signature at the call sites.
export type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

const FRAME_SUBJECT_TYPE = "frame";
const OCR_PROCESSOR = "ocr";

// The overlay's OCR lifecycle status:
//   "idle"     — no active frame / nothing requested.
//   "running"  — a job is queued or in-flight.
//   "success"  — completed job with ≥1 observation.
//   "empty"    — completed job with zero observations.
//   "missing"  — no OCR job/result has ever been recorded for this frame.
//   "error"    — fetch failed, the existing job is in failed state, or
//                its result payload is missing/invalid.
export type OcrStatus = "idle" | "running" | "success" | "empty" | "missing" | "error";

type OcrPayloadShape = {
  provider?: string;
  modelId?: string | null;
  observations?: OcrObservation[];
  provenance?: {
    provider?: string;
    modelId?: string | null;
  } | null;
};

type OcrJobPayloadShape = {
  provider?: string;
  modelId?: string | null;
};

export function formatOcrProviderLabel(provider: string): string {
  switch (provider) {
    case "apple_vision": return "Apple Vision";
    case "tesseract": return "Tesseract";
    case "paddle_ocr": return "PaddleOCR";
    default: return provider;
  }
}

export function resolveOcrProviderLabel(
  jobPayloadJson: string | null | undefined,
  resultPayloadJson: string | null | undefined,
  processorVersion: string | null | undefined,
): string | null {
  let jobPayload: OcrJobPayloadShape | null = null;
  let resultPayload: OcrPayloadShape | null = null;
  try {
    if (jobPayloadJson) jobPayload = JSON.parse(jobPayloadJson) as OcrJobPayloadShape;
  } catch {}
  try {
    if (resultPayloadJson) resultPayload = JSON.parse(resultPayloadJson) as OcrPayloadShape;
  } catch {}

  const provider =
    typeof resultPayload?.provider === "string"
      ? resultPayload.provider
      : typeof resultPayload?.provenance?.provider === "string"
        ? resultPayload.provenance.provider
        : typeof jobPayload?.provider === "string"
          ? jobPayload.provider
          : typeof processorVersion === "string" && processorVersion.includes(":")
            ? processorVersion.split(":", 1)[0]
            : processorVersion;
  if (!provider) return null;
  const modelId =
    typeof resultPayload?.modelId === "string"
      ? resultPayload.modelId
      : typeof resultPayload?.provenance?.modelId === "string"
        ? resultPayload.provenance.modelId
        : typeof jobPayload?.modelId === "string"
          ? jobPayload.modelId
          : null;
  const providerLabel = formatOcrProviderLabel(provider);
  return modelId ? `${providerLabel} · ${modelId}` : providerLabel;
}

export function parseOcrPayload(json: string | null | undefined): { observations: OcrObservation[]; providerLabel: string | null } | null {
  if (!json) return null;
  try {
    const parsed = JSON.parse(json) as Partial<OcrStructuredPayload>;
    const obs = Array.isArray(parsed?.observations) ? parsed.observations : null;
    if (!obs) return null;
    const out: OcrObservation[] = [];
    for (const o of obs) {
      const bb = o?.boundingBox;
      if (
        !bb ||
        typeof bb.x !== "number" ||
        typeof bb.y !== "number" ||
        typeof bb.width !== "number" ||
        typeof bb.height !== "number"
      )
        continue;
      out.push({
        text: typeof o.text === "string" ? o.text : "",
        confidence: typeof o.confidence === "number" ? o.confidence : 0,
        boundingBox: {
          x: bb.x,
          y: bb.y,
          width: bb.width,
          height: bb.height,
        },
      });
    }
    const provider =
      typeof parsed?.provider === "string"
        ? parsed.provider
        : typeof parsed?.provenance?.provider === "string"
          ? parsed.provenance.provider
          : null;
    const modelId =
      typeof parsed?.modelId === "string"
        ? parsed.modelId
        : typeof parsed?.provenance?.modelId === "string"
          ? parsed.provenance.modelId
          : null;
    return {
      observations: out,
      providerLabel: provider ? (modelId ? `${formatOcrProviderLabel(provider)} · ${modelId}` : formatOcrProviderLabel(provider)) : null,
    };
  } catch {
    return null;
  }
}

export type OcrLoadResult = {
  status: OcrStatus;
  observations: OcrObservation[];
  providerLabel: string | null;
  error: string | null;
  job: ProcessingJobDto | null;
};

export async function loadOcrFromJob(job: ProcessingJobDto, invoke: InvokeFn): Promise<OcrLoadResult> {
  if (job.status === "queued" || job.status === "running") {
    return {
      status: "running",
      observations: [],
      providerLabel: resolveOcrProviderLabel(job.payloadJson, null, null),
      error: null,
      job,
    };
  }
  if (job.status === "failed") {
    return {
      status: "error",
      observations: [],
      providerLabel: resolveOcrProviderLabel(job.payloadJson, null, null),
      error: job.lastError ?? "OCR job failed",
      job,
    };
  }

  const result = await invoke<ProcessingResultDto | null>("get_processing_result", {
    request: { jobId: job.id } satisfies GetProcessingResultRequest,
  });

  const parsedPayload = parseOcrPayload(result?.structuredPayloadJson);
  if (parsedPayload === null) {
    return {
      status: "error",
      observations: [],
      providerLabel: resolveOcrProviderLabel(job.payloadJson, result?.structuredPayloadJson, result?.processorVersion),
      error: result ? "OCR result payload is missing or invalid" : "OCR result not available",
      job,
    };
  }

  return {
    status: parsedPayload.observations.length === 0 ? "empty" : "success",
    observations: parsedPayload.observations,
    providerLabel: parsedPayload.providerLabel ?? resolveOcrProviderLabel(job.payloadJson, result?.structuredPayloadJson, result?.processorVersion),
    error: null,
    job,
  };
}

export async function loadOcrForFrame(sourceFrame: FrameDto, invoke: InvokeFn): Promise<OcrLoadResult> {
  const jobs = await invoke<ProcessingJobDto[]>("list_processing_jobs", {
    request: { subjectType: FRAME_SUBJECT_TYPE, subjectId: sourceFrame.id },
  });

  const ocrJobs = jobs.filter((j) => j.processor === OCR_PROCESSOR);
  if (ocrJobs.length === 0) {
    return { status: "missing", observations: [], providerLabel: null, error: null, job: null };
  }

  const completed = ocrJobs
    .filter((j) => j.status === "completed")
    .sort((a, b) => b.id - a.id);
  const job = completed[0] ?? ocrJobs.sort((a, b) => b.id - a.id)[0];
  return loadOcrFromJob(job, invoke);
}

// OCR box styles are expressed in PERCENTAGES of the overlay wrapper.
// The wrapper itself is sized/positioned to match the measured image
// rect (see template), so percentage coordinates inside it map 1:1 onto
// image-space coordinates. The lower-left origin of the source space
// means y must be flipped to CSS top.
//
// Boxes are drawn quietly by default; the recognized text only appears
// when the user hovers/focuses a single box. The reveal uses an opaque
// chip whose font-size is derived from the box height (so it visually
// matches the underlying glyph row and replaces — rather than doubles —
// the pixels underneath). `renderedImageHeight` is the measured pixel
// height of the rendered image rect (formerly a closed-over reactive var).
export function ocrBoxStyle(obs: OcrObservation, renderedImageHeight: number): string {
  const bb = obs.boundingBox;
  const leftPct = bb.x * 100;
  const topPct = (1 - bb.y - bb.height) * 100;
  const widthPct = bb.width * 100;
  const heightPct = bb.height * 100;
  // 0.78 ≈ cap-height ratio of common UI fonts; keeps glyphs vertically
  // centered inside the bbox without descenders escaping the bottom edge.
  const heightPx = Math.max(8, bb.height * renderedImageHeight);
  const fontSizePx = Math.max(6, heightPx * 0.78);
  return `left: ${leftPct}%; top: ${topPct}%; width: ${widthPct}%; height: ${heightPct}%; --ocr-font-size: ${fontSizePx.toFixed(2)}px;`;
}
