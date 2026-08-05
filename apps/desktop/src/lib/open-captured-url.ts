import { invoke } from "@tauri-apps/api/core";
import { toast } from "$lib/toast.svelte";

/** The three distinguishable outcomes of an open-captured-url attempt:
 *  - `opened`: the brokered command opened the page.
 *  - `no-url`: the command returned `false` — there was no openable http(s) URL
 *    behind this frame (URL was non-http(s), or the frame had none). A benign
 *    no-op, but worth a brief note instead of silence.
 *  - `error`: the brokered command threw (e.g. the opener plugin failed). */
export type OpenCapturedUrlStatus = "opened" | "no-url" | "error";

/** Outcome of an open-captured-url attempt, reported back to the caller so each
 *  surface can decide how (and whether) to show its own feedback. `error` is set
 *  only for the `"error"` status, stringified like the timeline's
 *  "Couldn't open URL: …". */
export type OpenCapturedUrlResult = {
  status: OpenCapturedUrlStatus;
  error?: string;
};

export type OpenCapturedUrlOptions = {
  /** Suppress the helper's built-in feedback dialog so the caller can render its
   *  own status (the dashboard's inline frame-action status). The result is
   *  still returned, so the caller can branch on `status`. */
  silent?: boolean;
};

// Feedback wording shared by both the no-url note and the error dialog, so every
// surface gets identical copy (mirroring quick-recall's keyboard ⌃O note).
const NO_URL_MESSAGE = "No openable page for this result.";
const FEEDBACK_TITLE = "Couldn't open page";

// Open the captured http(s) page behind a frame in the default browser via the
// brokered Rust command `open_captured_url`. The raw URL stays in Rust (only the
// guarded host+path is ever shown in the UI); this just hands the frame id to the
// broker, which resolves and opens it.
//
// Returns one of three statuses instead of swallowing the outcome: `"opened"`,
// `"no-url"` (the command returned false — a benign no-op), or `"error"` (the
// command threw, with `error` carrying the stringified failure). By default the
// helper itself surfaces the `no-url` and `error` cases as an app toast, so
// every card surface gets consistent feedback for free. Pass
// `{ silent: true }` to suppress that and render your own status (the dashboard
// does this — it has its own inline frame-action status line).
//
// Shared by every "open captured URL" affordance — SearchResultCard (search
// cards), Chat (answer-source cards), Quick Recall (search + answer-source
// cards), and the dashboard (active-frame actions menu) — so the invoke +
// outcome contract lives in exactly one place.
export async function openCapturedUrl(
  frameId: number,
  options: OpenCapturedUrlOptions = {},
): Promise<OpenCapturedUrlResult> {
  const result = await runOpenCapturedUrl(frameId);
  if (!options.silent) {
    surfaceOpenCapturedUrlResult(result);
  }
  return result;
}

// The bare invoke + outcome mapping, with no feedback. Split out so the feedback
// policy stays in one place and callers that need the raw outcome stay simple.
async function runOpenCapturedUrl(
  frameId: number,
): Promise<OpenCapturedUrlResult> {
  try {
    const opened = await invoke<boolean>("open_captured_url", { frameId });
    return { status: opened ? "opened" : "no-url" };
  } catch (err) {
    return {
      status: "error",
      error: typeof err === "string" ? err : "the page could not be opened",
    };
  }
}

// Surface a non-`opened` outcome as a toast so every caller shows the same copy:
// an info note for `no-url`, a persistent error for a real failure. Clicking a
// card should never be answered with a modal. A no-op on success.
function surfaceOpenCapturedUrlResult(result: OpenCapturedUrlResult): void {
  if (result.status === "no-url") {
    toast({ id: "open-captured-url", title: FEEDBACK_TITLE, message: NO_URL_MESSAGE });
    return;
  }
  if (result.status === "error") {
    toast({
      id: "open-captured-url",
      tone: "error",
      title: FEEDBACK_TITLE,
      message: `Couldn't open URL: ${result.error}`,
    });
  }
}
