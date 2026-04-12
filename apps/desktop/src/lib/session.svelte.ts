// Shared capture session state — used by both the layout and the dashboard
// page (for display / controls).
//
// All writes go through `setSession` instead of assigning
// `captureSession.value` directly. Inactivity detection is now handled
// natively by the backend (macOS system-wide idle); the frontend no longer
// needs to report window-level activity or guard against stale activity
// responses.

import type { CaptureSession } from "$lib/types";

const _state = $state<{ value: CaptureSession | null }>({ value: null });

/** Read the current session value (reactive). */
export const captureSession: { readonly value: CaptureSession | null } = {
  get value() { return _state.value; },
};

/**
 * Authoritative write — use for start, stop, and get_permissions responses.
 */
export function setSession(session: CaptureSession | null): void {
  _state.value = session;
}
