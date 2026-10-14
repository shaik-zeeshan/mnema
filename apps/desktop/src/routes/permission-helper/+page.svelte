<script lang="ts">
  // The permission helper window (`AppWindow::PermissionHelper`), floated by
  // `openPermissionHelper` beside the System Settings pane it just opened. It
  // polls the one permission it is about every second and closes itself a beat
  // after the switch flips. The window is a non-activating panel, so it never
  // takes focus from System Settings.
  import { invoke } from "@tauri-apps/api/core";
  import { page } from "$app/stores";
  import IconCheck from "~icons/lucide/check";
  import IconX from "~icons/lucide/x";
  import { closeCurrentWindow } from "$lib/surface-windows";
  import {
    PERMISSION_HELPER_LIST,
    permissionHelperGranted,
    permissionHelperKind,
  } from "$lib/shell/permission-helper";
  import type { BrowserUrlAccessibilityStatus, GetPermissionsResponse } from "$lib/types";

  const POLL_MS = 1000;
  // Long enough to read "On", short enough that it feels like it closed itself.
  const CLOSE_AFTER_GRANT_MS = 1400;

  const kind = $derived(permissionHelperKind($page.url.searchParams.get("kind")));
  let granted = $state(false);

  $effect(() => {
    const current = kind;
    granted = false;
    let closeTimer: ReturnType<typeof setTimeout> | undefined;

    async function check(): Promise<void> {
      try {
        const isGranted =
          current === "accessibility"
            ? permissionHelperGranted(
                current,
                null,
                (await invoke<BrowserUrlAccessibilityStatus>("get_browser_url_accessibility_status")).trusted,
              )
            : permissionHelperGranted(
                current,
                (await invoke<GetPermissionsResponse>("get_capture_permissions")).permissions,
                false,
              );
        if (!isGranted || closeTimer) return;
        granted = true;
        clearInterval(poll);
        closeTimer = setTimeout(() => void closeCurrentWindow(), CLOSE_AFTER_GRANT_MS);
      } catch {
        // A failed read is just "not yet": the next tick asks again.
      }
    }

    const poll = setInterval(check, POLL_MS);
    void check();
    return () => {
      clearInterval(poll);
      clearTimeout(closeTimer);
    };
  });
</script>

<div class="mx-helper" data-state={granted ? "granted" : "waiting"} role="status" aria-live="polite">
  <span class="mx-helper__icon"><img src="/app-icon.png" alt="" /></span>
  <div>
    <div class="mx-helper__title">Turn on Mnema in the {PERMISSION_HELPER_LIST[kind]} list</div>
    <div class="mx-helper__state">
      {#if granted}
        <IconCheck width="13" height="13" aria-hidden="true" />On — you can close System Settings
      {:else}
        <span class="mx-spin mx-spin--sm"></span>Waiting for the switch — this closes by itself
      {/if}
    </div>
  </div>
  <button
    type="button"
    class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
    aria-label="Dismiss"
    onclick={() => void closeCurrentWindow()}
  >
    <IconX width="14" height="14" aria-hidden="true" />
  </button>
</div>

<style>
  /* The card IS the window. The kit draws it fixed bottom-centre inside a page
     (the mockup's stand-in); here Rust puts the window there instead, and the
     native window shadow replaces the CSS one a 62px-tall window would clip. */
  .mx-helper {
    position: static;
    translate: none;
    width: 100%;
    box-shadow: none;
  }
</style>
