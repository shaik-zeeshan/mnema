// "view frame →" / "view in Timeline →" — the one hand-off both the list
// inspector and the story spine use. Hands the Activity's first raw evidence
// ref to the main window; when nothing precise resolves it says so before
// falling back to the Timeline, so landing at the top doesn't read as the wrong
// moment opening.
import { invoke } from "@tauri-apps/api/core";
import { goto } from "$app/navigation";
import { toast } from "$lib/toast.svelte";
import type { ActivityEvidenceRef } from "$lib/types/recording";

export async function openEvidenceRef(
  ref: ActivityEvidenceRef | undefined,
): Promise<void> {
  try {
    if (ref?.subjectType === "audio_segment") {
      await invoke("open_capture_result_in_main_window", {
        kind: "audio",
        frameId: null,
        audioSegmentId: ref.subjectId,
        // spanStartMs is an offset WITHIN the segment, not an absolute time.
        spanStartMs: null,
        alignedFrameId: null,
      });
      return;
    }
    if (ref?.subjectType === "frame") {
      await invoke("open_capture_result_in_main_window", {
        kind: "frame",
        frameId: ref.subjectId,
        audioSegmentId: null,
      });
      return;
    }
  } catch {
    // fall through to a plain Timeline navigation
  }
  toast({ title: "Opening Timeline", message: "Couldn't pinpoint the exact moment." });
  void goto("/");
}
