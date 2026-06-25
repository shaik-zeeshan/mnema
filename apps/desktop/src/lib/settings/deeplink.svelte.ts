// Settings deeplink transport, extracted out of the Main window's
// `+layout.svelte` shell so the shell stops absorbing this concern.
//
// The Main window owns the `/settings` route, so it is the single place that
// turns an `open_settings_tab` deeplink (emitted by Rust's
// `focus_main_and_open_settings` for the tray and other windows) into a route
// navigation. There are two doors:
//
//   1. Live event: a warm Main window already has a listener attached, so the
//      `open_settings_tab` event navigates immediately.
//   2. Cold-window drain: a freshly-built Main window boots on Timeline (`/`),
//      and the live event fires from Rust before the listener attaches — Tauri
//      drops an event with no listener. Rust queues the normalized payload only
//      when Main had to be built, so we drain (consume) that queue once on mount
//      and navigate if a deeplink is pending.
//
// The settings page reacts to the resulting `?tab`/`?focus` query reactively,
// so this is the one navigation — no double-handling.
//
// This is a `.svelte.ts` module because it is wired from the layout's lifecycle.
// It contains no runes of its own (the layout keeps owning the `$effect` and the
// one-shot cold-drain gate, so the insights peek and the settings drain stay
// gated together exactly as before); it only needs reactive *reads* of the
// caller's live shell state, which it takes as getter functions so each handler
// observes the current value at invocation time rather than a stale snapshot.

import type { goto } from "$app/navigation";
import {
  recordMainSurface,
  settingsRoutePath,
  type SettingsWindowTab,
  type SettingsWindowFocus,
} from "$lib/surface-windows";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

// Wire payload for the `open_settings_tab` event and the
// `drain_pending_open_settings` command. `tab`/`focus` arrive as raw strings off
// the wire and are narrowed to the canonical types by `settingsRoutePath`'s
// normalizer, mirroring the previous inline casts in the layout.
type SettingsDeeplinkPayload = { tab?: string; focus?: string };

export interface SettingsDeeplinkDeps {
  /** Live read of the current route pathname (the layout passes
   *  `() => $page.url.pathname`). A getter — not a snapshot — so each handler
   *  observes the route at invocation time, preserving the reactivity the
   *  inline `$page.url.pathname` reads had in the layout. */
  currentPathname: () => string;
  /** `$app/navigation`'s `goto`, threaded in rather than imported, to match how
   *  the layout closes over navigation. */
  goto: typeof goto;
  /** Live shell predicate: is this the Main window (not a dedicated/panel one)? */
  isMainWindow: () => boolean;
  /** Live shell predicate: is the current route already on `/settings`? */
  isSettings: () => boolean;
}

export interface SettingsDeeplink {
  /**
   * Register the live `open_settings_tab` listener. Returns a cleanup function
   * that tears the listener down — call it on unmount / `$effect` cleanup, the
   * same way the layout's combined `$effect` did. Idempotent teardown: the
   * returned cleanup is safe to call even if the listener never finished
   * attaching (it flips a local `destroyed` flag the async `.then` honors).
   */
  listen: () => () => void;
  /**
   * Drain (consume) the cold-window settings queue exactly once and navigate if
   * a deeplink is pending. Must be invoked by the caller under its single
   * one-shot gate (the layout's `coldDrainsDone` flag) so it stays sequenced
   * with the insights peek precisely as before.
   *
   * `isActive` mirrors the `!destroyed` check the inline drain made inside its
   * resolved `.then`: it bails if the owning `$effect` run was torn down before
   * the async drain resolved (the same flag the listener cleanup flips). The
   * `isSettings` guard still skips a redundant navigation when already on
   * `/settings`.
   */
  drainColdWindow: (isActive: () => boolean) => void;
}

export function createSettingsDeeplink(deps: SettingsDeeplinkDeps): SettingsDeeplink {
  const { currentPathname, goto, isMainWindow, isSettings } = deps;

  function navigateToSettings(payload: SettingsDeeplinkPayload | undefined): void {
    // Remember the main surface we're leaving so the settings rail's "← Back to
    // app" returns there instead of a stale path. The in-window `openSettings`
    // helper does this for its own caller, but a deeplink (tray / Quick Recall)
    // navigates here directly without going through it. `recordMainSurface`
    // no-ops for any non-main path, so calling it unconditionally is safe.
    recordMainSurface(currentPathname());
    void goto(
      settingsRoutePath(
        payload?.tab as SettingsWindowTab | undefined,
        payload?.focus as SettingsWindowFocus | undefined,
      ),
    );
  }

  function listenForOpenSettings(): () => void {
    let destroyed = false;
    let unlisten: (() => void) | undefined;

    void listen<SettingsDeeplinkPayload>("open_settings_tab", (event) => {
      if (destroyed || !isMainWindow()) return;
      navigateToSettings(event.payload);
    }).then((fn) => {
      if (destroyed) fn();
      else unlisten = fn;
    });

    return () => {
      destroyed = true;
      unlisten?.();
    };
  }

  function drainColdWindow(isActive: () => boolean): void {
    // Cold-window Settings deeplink drain. A freshly-built main window (cold-start
    // tray "Open Settings") boots on Timeline, and the live `open_settings_tab`
    // event fires from Rust before the listener has attached — Tauri drops an
    // event with no listener, so without this drain the user would be stranded on
    // Timeline. Rust queues the normalized payload only when Main had to be built,
    // so a warm window's queue is empty here.
    if (!isMainWindow() || isSettings()) return;
    void invoke<SettingsDeeplinkPayload[]>("drain_pending_open_settings")
      .then((payloads) => {
        const next = payloads?.[payloads.length - 1];
        if (!isActive() || !next) return;
        // The drain CONSUMES the Rust queue, so the payload is already gone and
        // unrecoverable. Unlike the non-consuming insights peek, we cannot bail on
        // a mid-drain navigation without losing the deeplink forever — so we honor
        // it even after a same-tick navigation. The `isSettings` guard still skips
        // a redundant navigation when we're already on /settings; the user reached
        // a cold-built window expecting Settings, so navigating there is correct.
        if (isSettings()) return;
        navigateToSettings(next);
      })
      .catch(() => {
        // Best-effort: leave the route as-is if the drain is unavailable.
      });
  }

  return {
    listen: listenForOpenSettings,
    drainColdWindow,
  };
}
