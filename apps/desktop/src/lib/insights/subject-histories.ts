// Confidence History for a set of subjects (get_user_context_subject), shared
// by Subjects' spark lines and Overview's "What changed" rows.
import { invoke } from "@tauri-apps/api/core";
import type { ConfidenceSnapshot, SubjectView } from "$lib/types/recording";

/** subject → (conclusionId → oldest-first history). Bounded concurrency (4)
 * keeps a large dossier responsive; a failed fetch just leaves that subject out. */
export async function loadSubjectHistories(
  subjects: Iterable<string>,
): Promise<Map<string, Map<number, ConfidenceSnapshot[]>>> {
  const queue = [...new Set(subjects)];
  const out = new Map<string, Map<number, ConfidenceSnapshot[]>>();
  let cursor = 0;
  async function worker(): Promise<void> {
    while (cursor < queue.length) {
      const subject = queue[cursor++];
      try {
        const view = await invoke<SubjectView>("get_user_context_subject", { subject });
        out.set(subject, new Map(view.trajectories.map((t) => [t.conclusionId, t.history])));
      } catch {
        // Best-effort.
      }
    }
  }
  await Promise.all(Array.from({ length: Math.min(4, queue.length) }, worker));
  return out;
}
