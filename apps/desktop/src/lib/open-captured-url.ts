import { invoke } from "@tauri-apps/api/core";

// Open the captured http(s) page behind a frame in the default browser via the
// brokered Rust command `open_captured_url`. The raw URL stays in Rust (only the
// guarded host+path is ever shown in the UI); this just hands the frame id to the
// broker, which resolves and opens it. Best-effort: a missing/unopenable URL
// simply does nothing, so callers can `void` this without their own try/catch.
//
// Shared by every "open captured URL" affordance — SearchResultCard (search
// cards), Chat (answer-source cards), and Quick Recall (search + answer-source
// cards) — so the invoke + swallow-on-failure contract lives in exactly one place.
export async function openCapturedUrl(frameId: number): Promise<void> {
  try {
    await invoke("open_captured_url", { frameId });
  } catch {
    // Best-effort: a missing/unopenable URL simply does nothing.
  }
}
