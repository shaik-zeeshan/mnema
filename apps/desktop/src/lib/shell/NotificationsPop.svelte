<script lang="ts">
  // Titlebar bell + notifications popover (SHELL.md › Notifications). Moved out
  // of +layout.svelte; the store, actions, ages, focus return and live regions
  // are unchanged — only the skin is the kit's `mx-pop mx-notifs`.
  import { tick } from "svelte";
  import { tip } from "$lib/components/tooltip";
  import { getFocusableElements, trapTabKey } from "$lib/keyboard";
  import { openSettings } from "$lib/surface-windows";
  import {
    appNotifications,
    clearAppNotification,
    clearAppNotifications,
    dismissAppNotificationError,
    noteAppNotificationError,
    reloadAppNotifications,
    type AppNotification,
  } from "$lib/notifications.svelte";

  let open = $state(false);
  let openedByKeyboard = false;
  let buttonEl = $state<HTMLButtonElement | null>(null);
  let popEl = $state<HTMLDivElement | null>(null);

  const count = $derived(appNotifications.count);
  const loadError = $derived(appNotifications.loadError);
  const actionError = $derived(appNotifications.actionError);
  const hasError = $derived(appNotifications.items.some((n) => n.severity === "error"));
  const hasWarning = $derived(appNotifications.items.some((n) => n.severity === "warning"));
  // Badge = worst unread severity; no notifications (and no load failure) = no badge.
  const badgeTone = $derived(
    hasError || loadError !== null ? "danger" : hasWarning ? "warn" : undefined,
  );
  const showBadge = $derived(count > 0 || loadError !== null);

  // The badge is decorative, so the live summary goes into the button name and
  // two always-mounted live regions (assertive when an error is present).
  const summary = $derived.by<string>(() => {
    if (loadError) return "Notifications failed to load — open to retry.";
    if (count === 0) return "";
    const noun = count === 1 ? "notification" : "notifications";
    const severity = hasError ? ", including an error" : hasWarning ? ", including a warning" : "";
    return `${count} ${noun}${severity}`;
  });
  const liveTone = $derived(hasError || loadError !== null ? "assertive" : "polite");

  const toneOf = (n: AppNotification) =>
    n.severity === "error" ? "danger" : n.severity === "warning" ? "warn" : "info";

  function actionLabel(n: AppNotification): string {
    if (n.action?.type !== "open_settings_tab") return "Open";
    if (n.action.tab === "about") return "Open update settings";
    if (n.action.tab === "processing") return "Open OCR settings";
    if (n.action.tab === "transcription") return "Open transcription settings";
    if (n.action.tab === "speakers") return "Open speaker settings";
    if (n.action.tab === "shortcuts") return "Open shortcut settings";
    return "Open settings";
  }

  async function runAction(n: AppNotification): Promise<void> {
    if (n.action?.type !== "open_settings_tab") return;
    try {
      await openSettings(n.action.tab);
    } catch {
      // Keep the row and the popover so the user sees the action didn't complete.
      noteAppNotificationError("Couldn't open settings. Try again.");
      return;
    }
    if (await clearAppNotification(n.id)) open = false;
  }

  // Relative age per row, re-evaluated every 30 s while the popover is open.
  let now = $state(Date.now());
  $effect(() => {
    if (!open) return;
    now = Date.now();
    const handle = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(handle);
  });

  function age(createdAtUnixMs: number): string {
    const seconds = Math.floor(Math.max(0, now - createdAtUnixMs) / 1000);
    if (seconds < 45) return "just now";
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.round(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    return `${Math.round(hours / 24)}d ago`;
  }

  function stamp(createdAtUnixMs: number): string {
    try {
      return new Date(createdAtUnixMs).toLocaleString();
    } catch {
      return "";
    }
  }

  function onWindowPointerDown(event: PointerEvent): void {
    if (!open) return;
    const target = event.target as Node | null;
    if (!target || popEl?.contains(target) || buttonEl?.contains(target)) return;
    open = false;
  }

  function onWindowKeydown(event: KeyboardEvent): void {
    if (!open || event.key !== "Escape" || event.defaultPrevented) return;
    event.preventDefault();
    event.stopPropagation();
    open = false;
  }

  // Keyboard-opened: focus the first control; on close, return focus to the bell.
  $effect(() => {
    if (!open) return;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled || !open || !openedByKeyboard) return;
      getFocusableElements(popEl)[0]?.focus({ preventScroll: true });
    });
    return () => {
      cancelled = true;
      const active = document.activeElement as HTMLElement | null;
      if ((openedByKeyboard && (!active || active === document.body)) || (active && popEl?.contains(active))) {
        buttonEl?.focus({ preventScroll: true });
      }
      openedByKeyboard = false;
    };
  });
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeydown} />

<span class="mx-sr" aria-live="polite" aria-atomic="true">{liveTone === "polite" ? summary : ""}</span>
<span class="mx-sr" aria-live="assertive" aria-atomic="true">{liveTone === "assertive" ? summary : ""}</span>
<div class="mx-pop-anchor">
  <button
    bind:this={buttonEl}
    type="button"
    class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
    aria-label={summary ? `Notifications — ${summary}` : "Notifications"}
    aria-expanded={open}
    aria-controls="notification-popover"
    use:tip={"Notifications"}
    onkeydown={(e) => (e.key === "Enter" || e.key === " ") && (openedByKeyboard = true)}
    onpointerdown={() => (openedByKeyboard = false)}
    onclick={() => (open = !open)}
  >
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 8a6 6 0 1 1 12 0c0 7 3 9 3 9H3s3-2 3-9" /><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" /></svg>
    {#if showBadge}<span class="mx-badge" data-tone={badgeTone} aria-hidden="true"></span>{/if}
  </button>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    id="notification-popover"
    class="mx-pop mx-notifs"
    data-open={open ? "" : undefined}
    role="dialog"
    aria-label="Notifications"
    aria-hidden={!open}
    inert={!open}
    tabindex="-1"
    bind:this={popEl}
    onkeydown={(e) => trapTabKey(e, popEl)}
  >
    <div class="mx-notifs__head">
      <span class="mx-kicker">Notifications</span>
      {#if count > 0}<span class="num">{count}</span>{/if}
      <span class="mx-spacer"></span>
      <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" data-clear-all onclick={() => void clearAppNotifications()}>Clear all</button>
    </div>
    {#if loadError}
      <div class="mx-notifs__error" role="alert">
        <span class="mx-inline" data-tone="danger">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" aria-hidden="true"><circle cx="12" cy="12" r="9" /><path d="M12 8v5M12 16h.01" /></svg>
          Couldn’t load notifications.
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" use:tip={loadError} onclick={() => void reloadAppNotifications()}>Retry</button>
        </span>
      </div>
    {/if}
    {#if actionError}
      <div class="mx-notifs__error" role="alert">
        <span class="mx-inline" data-tone="danger">
          {actionError}
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={dismissAppNotificationError}>Dismiss</button>
        </span>
      </div>
    {/if}
    <ul class="mx-notifs__list">
      {#each appNotifications.items as n (n.id)}
        <li class="mx-notif" data-tone={toneOf(n)}>
          <b class="mx-notif__title">{n.title}</b>
          <time class="mx-notif__time" datetime={new Date(n.createdAtUnixMs).toISOString()} use:tip={stamp(n.createdAtUnixMs)}>{age(n.createdAtUnixMs)}</time>
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm mx-notif__x" aria-label="Dismiss notification" onclick={() => void clearAppNotification(n.id)}>
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12" /></svg>
          </button>
          <p class="mx-notif__text">{n.message}</p>
          {#if n.action?.type === "open_settings_tab"}
            <div class="mx-notif__acts">
              <button type="button" class="mx-btn mx-btn--sm" onclick={() => void runAction(n)}>{actionLabel(n)}</button>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
    {#if !loadError}
    <div class="mx-empty mx-empty--compact mx-notifs__empty">
      <span class="mx-empty__glyph">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 8a6 6 0 1 1 12 0c0 7 3 9 3 9H3s3-2 3-9" /><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" /></svg>
      </span>
      <b class="mx-empty__title">You’re all caught up</b>
      <p class="mx-empty__text">Capture, model and update warnings land here.</p>
    </div>
    {/if}
  </div>
</div>
