// Delete Recent Capture — the pure half: the three windows the backend accepts
// and the words for each. The confirm text is the menu bar's, verbatim
// (`confirm_delete_recent` in `src-tauri/src/status_bar.rs`), so both doors ask
// the same question.

/** The only windows `delete_recent_capture` is ever called with. */
export const DELETE_WINDOW_SECONDS = [60, 300, 900] as const;
export type DeleteWindowSeconds = (typeof DELETE_WINDOW_SECONDS)[number];

export const DELETE_WINDOW_OPTIONS = DELETE_WINDOW_SECONDS.map((s) => ({
  value: String(s),
  label: `${s / 60} min`,
}));

/** "last 5 minutes" — the tray's phrase for a window. */
export function deleteWindowPhrase(seconds: DeleteWindowSeconds): string {
  const minutes = seconds / 60;
  return `last ${minutes} minute${minutes === 1 ? "" : "s"}`;
}

export function deleteConfirmMessage(seconds: DeleteWindowSeconds): string {
  return `Delete the ${deleteWindowPhrase(seconds)} from Mnema's library? This removes whole overlapping capture segments and cannot be undone.`;
}

export interface DeleteReceipt {
  tone: "ok" | "warn";
  text: string;
}

/** The one-line receipt under the row (the tray gives none). */
export function deleteReceipt(seconds: DeleteWindowSeconds, fileDeleteErrors: number): DeleteReceipt {
  const done = `Deleted the ${deleteWindowPhrase(seconds)}`;
  if (fileDeleteErrors <= 0) return { tone: "ok", text: `${done}.` };
  const files = `${fileDeleteErrors} file${fileDeleteErrors === 1 ? "" : "s"}`;
  return { tone: "warn", text: `${done}, but ${files} couldn’t be removed from disk yet — Mnema will retry.` };
}
