import { invoke } from "@tauri-apps/api/core";

/** Outcome of an open-captured-url attempt, reported back to the caller so each
 *  surface can decide how (and whether) to show feedback. `opened` is the Rust
 *  command's boolean — false with no `error` means "no openable http(s) URL for
 *  this frame" (a benign no-op); `error` is set only when the brokered command
 *  actually threw (e.g. the opener plugin failed), stringified like the timeline. */
export type OpenCapturedUrlResult = { opened: boolean; error?: string };

// Open the captured http(s) page behind a frame in the default browser via the
// brokered Rust command `open_captured_url`. The raw URL stays in Rust (only the
// guarded host+path is ever shown in the UI); this just hands the frame id to the
// broker, which resolves and opens it.
//
// Returns the outcome instead of swallowing it: `{opened}` carries the command's
// boolean (false = no openable URL, a benign no-op), and a thrown command error
// comes back as `{opened: false, error}` so the caller can surface a real failure
// the way its surface already shows transient status (mirroring the timeline's
// "Couldn't open URL: …"). Callers may still ignore the result for the no-error
// no-op case, but should report the `error` case.
//
// Shared by every "open captured URL" affordance — SearchResultCard (search
// cards), Chat (answer-source cards), and Quick Recall (search + answer-source
// cards) — so the invoke + outcome contract lives in exactly one place.
export async function openCapturedUrl(
  frameId: number,
): Promise<OpenCapturedUrlResult> {
  try {
    const opened = await invoke<boolean>("open_captured_url", { frameId });
    return { opened };
  } catch (err) {
    return {
      opened: false,
      error: typeof err === "string" ? err : "the page could not be opened",
    };
  }
}
