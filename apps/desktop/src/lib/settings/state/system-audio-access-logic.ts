// Pure hint-visibility rules for the system-audio access store (ADR 0052).
// Extracted from `system-audio-access.svelte.ts` (a runes module bun:test can't
// import) so the decision is testable in isolation; the store delegates here.

/** Visible only while the backend says show AND the user hasn't dismissed it. */
export function systemAudioHintVisible(
  hint: { shouldShow: boolean } | null,
  dismissed: boolean,
): boolean {
  return (hint?.shouldShow ?? false) && !dismissed;
}

/** A dismissal is recordable only when the hint carries its prompt id. */
export function canDismissSystemAudioHint(
  hint: { promptId?: string | null } | null,
): boolean {
  return Boolean(hint?.promptId);
}
