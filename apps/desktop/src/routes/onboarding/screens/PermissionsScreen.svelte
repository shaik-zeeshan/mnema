<!--
  Screen 2 / 8 — Permissions.  (issue #195, slice 6)

  CONTRACT
    props
      flow        OnboardingFlow. Reads, all via `flow.controller`:
                    permissions              Record<"screen"|"microphone"|"systemAudio", PermissionValue> | null
                    requestingPerm           the key currently being requested, or null
                    refreshingPerms          boolean
                    sysAudioPromptRaised     boolean — the OS prompt has been raised at least once
                    permissionAction/Label/Tone(value)  display helpers (onboarding-attention.ts)
                    requestPermission(key) / refreshPermissions()
                    geckoInstalled, geckoTrusted, geckoInstalledNames,
                    requestGeckoAccess(), openGeckoAccessSettings(), recheckGeckoAccess()
      onContinue  () => void — advance to Capture & Storage. The shell RE-RUNS THE
                  RESOLVER on this transition, so a grant made here reaches the
                  settings. Nothing else needs to be signalled.
      onBack      () => void — return to Welcome.
    emits
      onContinue / onBack only. Permission requests go straight to the controller.
    owns
      One request on screen at a time, Screen Recording first. Denial recovery
      (deep-link to the right System Settings pane + re-check). The relaunch
      offer after granting Screen Recording. Accessibility (browser URLs) last,
      optional, and only when a Gecko browser is installed.
    must not
      Gate anything — macOS never re-prompts after a denial, so a hard gate here
      would trap the user with no in-app recovery. Never show system audio as
      confirmed: its grant cannot be read (ADR 0052), so it gets Request /
      Request again and a plain statement that macOS will not confirm it.
    gates
      None. Continue is always live.

  SHAPE: the screen is a sequence of MOMENTS, not a list of rows. One question is
  asked at a time; every answered question shrinks to a single ledger line above.
  A moment is answered when it is granted (nothing more to ask) or when the user
  presses Continue past it — that single rule is what makes a denial, a closed
  system-audio prompt and a skipped Accessibility offer all behave identically.
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";
  import type { PermissionKey, PermissionValue } from "../onboarding-attention";

  let {
    flow,
    onContinue,
    onBack,
  }: { flow: OnboardingFlow; onContinue: () => void; onBack: () => void } = $props();

  type MomentId = PermissionKey | "accessibility";

  /** Bar silhouette for the level meter (mockup 02b). Shape only — see the
   *  ponytail note in the markup for why it carries no live amplitude. */
  const BAR_HEIGHTS = [12, 22, 15, 31, 19, 26, 36, 17, 28, 21, 33, 14, 24, 30, 16, 23, 11, 18, 27, 13];

  const NAMES: Record<MomentId, string> = {
    screen: "Screen recording",
    microphone: "Microphone",
    systemAudio: "System audio",
    accessibility: "Accessibility",
  };

  const c = $derived(flow.controller);
  const value = (key: PermissionKey): PermissionValue | undefined => c.permissions?.[key];
  const isGranted = (v: PermissionValue | undefined): boolean =>
    v === "granted" || v === "assumed_working" || v === "unsupported";
  const isDenied = (v: PermissionValue | undefined): boolean =>
    v === "denied" || v === "restricted";

  /** Moments the user has continued past. A denial can never be un-denied here. */
  let passed = $state<MomentId[]>([]);
  /** Screen Recording granted *in this process* — macOS only honours it after a relaunch. */
  let screenNeedsRelaunch = $state(false);
  /** The Accessibility prompt has been raised, so the recovery pair replaces Request. */
  let accessibilityAsked = $state(false);

  // Accessibility is offered last, and only when a Gecko browser (Firefox/Zen)
  // is installed — every other browser reports its URL without it (ADR 0039).
  const order = $derived<MomentId[]>(
    c.geckoInstalled
      ? ["screen", "microphone", "systemAudio", "accessibility"]
      : ["screen", "microphone", "systemAudio"],
  );

  function answered(id: MomentId): boolean {
    if (passed.includes(id)) return true;
    if (id === "accessibility") return c.geckoTrusted;
    // System audio can never answer itself: there is no grant to read (ADR
    // 0052), and even sound arriving is evidence to show the user, not a reason
    // to skip past the moment that shows it. Only a Continue settles it.
    if (id === "systemAudio") return false;
    return isGranted(value(id));
  }

  const current = $derived(order.find((id) => !answered(id)) ?? null);
  const settled = $derived(order.filter((id) => id !== current && answered(id)));
  const upcoming = $derived(
    current === null ? [] : order.slice(order.indexOf(current) + 1).filter((id) => !answered(id)),
  );
  const isLast = $derived(current === null || upcoming.length === 0);
  const nothingGranted = $derived(c.grantedCount === 0 && !c.geckoTrusted);

  const soundArriving = $derived(value("systemAudio") === "assumed_working");

  function ledgerValue(id: MomentId): string {
    if (id === "accessibility") return c.geckoTrusted ? "granted" : "skipped";
    const v = value(id);
    if (id === "systemAudio") {
      if (soundArriving) return "sound is arriving";
      return c.sysAudioPromptRaised ? "requested · unconfirmed" : "not requested";
    }
    if (isGranted(v)) {
      return id === "screen" && screenNeedsRelaunch ? "granted · needs one relaunch" : "granted";
    }
    if (isDenied(v)) return "denied";
    return "not requested";
  }

  // System audio NEVER gets the tick, not even when sound is arriving: a green
  // check would claim a grant nothing on this machine can read (ADR 0052).
  const ledgerGranted = (id: MomentId): boolean =>
    id === "accessibility" ? c.geckoTrusted : id !== "systemAudio" && isGranted(value(id));

  async function request(key: PermissionKey): Promise<void> {
    const before = isGranted(value(key));
    await c.requestPermission(key);
    // macOS hands ScreenCaptureKit the new grant only to a fresh process, and
    // there is no backend signal for that — a grant that appeared during this
    // session IS the signal.
    if (key === "screen" && !before && isGranted(value("screen"))) screenNeedsRelaunch = true;
  }

  async function askAccessibility(): Promise<void> {
    accessibilityAsked = true;
    await c.requestGeckoAccess();
  }

  function advance(): void {
    if (isLast) {
      onContinue();
      return;
    }
    if (current !== null) passed = [...passed, current];
  }

  const primaryLabel = $derived.by(() => {
    if (isLast) return nothingGranted ? "Continue with nothing granted" : "Capture & Storage";
    if (current !== null && current !== "accessibility" && isDenied(value(current))) {
      return `Continue without ${current === "screen" ? "the screen" : "the microphone"}`;
    }
    return "Continue";
  });
</script>

<span class="ob-m">One at a time · none of them required</span>

{#if settled.length > 0}
  <div class="settled">
    {#each settled as id (id)}
      <div class="s-row">
        <span class="tk" class:on={ledgerGranted(id)} aria-hidden="true">{ledgerGranted(id) ? "✓" : "○"}</span>
        <span class="k">{NAMES[id]}</span>
        <span class="v">
          {ledgerValue(id)}
          {#if id === "screen" && screenNeedsRelaunch}
            <button class="ob-btn sm" onclick={() => invoke("request_app_relaunch")}>Relaunch</button>
          {/if}
        </span>
      </div>
    {/each}
  </div>
{/if}

<div class="moment">
  {#if current === "screen"}
    {#if isDenied(value("screen"))}
      <h1 class="ob-disp mid">Screen recording</h1>
      <p class="ob-lead">You said no. Mnema keeps working — but it will not record your screen.</p>
      <div class="ob-acts">
        <button class="ob-btn" onclick={() => request("screen")}>Open System Settings&nbsp; ›</button>
        <button class="ob-btn" disabled={c.refreshingPerms} onclick={() => c.refreshPermissions()}>
          {c.refreshingPerms ? "Checking…" : "Re-check"}
        </button>
        <span class="ob-fine">Privacy &amp; Security → Screen Recording → Mnema</span>
      </div>
      <p class="ob-fine tail">macOS never asks again on its own, which is why this screen never blocks.</p>
    {:else}
      <h1 class="ob-disp mid">Your screen.</h1>
      <p class="ob-lead">The one Mnema is built on — it records the screen so you can scrub back to it.</p>
      <div class="ob-acts">
        <button class="ob-btn" disabled={c.requestingPerm !== null} onclick={() => request("screen")}>
          {c.requestingPerm === "screen" ? "Requesting…" : "Request access"}
        </button>
        <span class="ob-fine">macOS raises its own prompt. Nothing is recorded until you finish setup.</span>
      </div>
    {/if}
  {:else if current === "microphone"}
    {#if isDenied(value("microphone"))}
      <h1 class="ob-disp mid">Microphone</h1>
      <p class="ob-lead">You said no. Mnema keeps working — without transcripts or speaker names.</p>
      <div class="ob-acts">
        <button class="ob-btn" onclick={() => request("microphone")}>Open System Settings&nbsp; ›</button>
        <button class="ob-btn" disabled={c.refreshingPerms} onclick={() => c.refreshPermissions()}>
          {c.refreshingPerms ? "Checking…" : "Re-check"}
        </button>
        <span class="ob-fine">Privacy &amp; Security → Microphone → Mnema</span>
      </div>
      <p class="ob-fine tail">macOS never asks again on its own, which is why this screen never blocks.</p>
    {:else}
      <h1 class="ob-disp mid">Your microphone.</h1>
      <p class="ob-lead">Only what you say, transcribed on this Mac — no audio ever leaves it.</p>
      <div class="ob-acts">
        <button class="ob-btn" disabled={c.requestingPerm !== null} onclick={() => request("microphone")}>
          {c.requestingPerm === "microphone" ? "Requesting…" : "Request access"}
        </button>
        <span class="ob-fine">Off by itself, this only costs you transcripts and speaker names.</span>
      </div>
    {/if}
  {:else if current === "systemAudio"}
    <h1 class="ob-disp mid">Play anything.</h1>
    <p class="ob-lead">macOS will not tell us whether you granted this — sound arriving is the only proof.</p>
    <div class="level">
      <!-- ponytail: the bars resolve to arriving / not arriving, not to a live
           level. `system_audio_activity_level()` exists in capture-system-audio
           but is fed only by a RUNNING tap, and onboarding starts no capture —
           the permission prompt's own tap is a 250 ms throwaway with a no-op
           callback. Animating them here would be theatre, not evidence. Upgrade
           path: a listening tap + a `get_system_audio_activity_level` command,
           the sibling of `get_microphone_activity_level`. -->
      <div class="meter" class:lit={soundArriving} aria-hidden="true">
        {#each BAR_HEIGHTS as h, i (i)}
          <i style="height:{soundArriving ? h : 6}px"></i>
        {/each}
      </div>
      <div>
        <div class="ob-strong">
          {#if soundArriving}
            Sound is arriving.
          {:else if c.sysAudioPromptRaised}
            Requested · unconfirmed
          {:else}
            Nothing has arrived yet
          {/if}
        </div>
        {#if !soundArriving}
          <p class="ob-fine">
            {c.sysAudioPromptRaised ? "Nothing has arrived yet · " : ""}Silent is not denied — it
            might just be a quiet Mac.
          </p>
        {/if}
      </div>
    </div>
    <div class="ob-acts">
      <button class="ob-btn" disabled={c.requestingPerm !== null} onclick={() => request("systemAudio")}>
        {#if c.requestingPerm === "systemAudio"}
          Requesting…
        {:else}
          {c.sysAudioPromptRaised ? "Request again" : "Request"}
        {/if}
      </button>
      <span class="ob-fine">A prompt you closed by accident looks exactly like a No.</span>
    </div>
  {:else if current === "accessibility"}
    <h1 class="ob-disp mid">Browser page addresses.</h1>
    <p class="ob-lead">
      Optional, and only for {c.geckoInstalledNames.join(" and ") || "Firefox and Zen"} — every other
      browser tells Mnema its page address already.
    </p>
    <div class="ob-acts">
      {#if accessibilityAsked}
        <button class="ob-btn" onclick={() => c.openGeckoAccessSettings()}>Open System Settings&nbsp; ›</button>
        <button class="ob-btn" disabled={c.recheckingGeckoAccess} onclick={() => c.recheckGeckoAccess()}>
          {c.recheckingGeckoAccess ? "Checking…" : "Re-check"}
        </button>
      {:else}
        <button class="ob-btn" disabled={c.requestingGeckoAccess} onclick={askAccessibility}>
          {c.requestingGeckoAccess ? "Requesting…" : "Request access"}
        </button>
      {/if}
      <span class="ob-fine">Privacy &amp; Security → Accessibility → Mnema</span>
    </div>
    <p class="ob-fine tail">The broadest grant there is, so it never rides along with the others.</p>
  {:else}
    <h1 class="ob-disp mid">That is everything.</h1>
    <p class="ob-lead">Nothing left to ask — all of it stays changeable in Settings.</p>
  {/if}
</div>

<div class="ob-foot">
  <hr class="ob-rule" />
  <div class="ob-acts">
    <div class="spacer trail">
      <button class="ob-btn ghost" onclick={onBack}>← Back</button>
      {#if upcoming.length > 0}
        <span class="upcoming">
          {#each upcoming as id, i (id)}
            <span>
              {i === 0 ? "next" : "then"} · {NAMES[id]}
              {#if id === "accessibility"}<span class="faint">(optional)</span>{/if}
            </span>
          {/each}
        </span>
      {:else if nothingGranted}
        <span class="ob-fine">Mnema will run, but it will not record anything.</span>
      {/if}
    </div>
    <button class="ob-btn primary" onclick={advance}>{primaryLabel}&nbsp; →</button>
  </div>
</div>

<style>
  /* settled ledger — one line per answered permission */
  .settled {
    display: flex;
    flex-direction: column;
    margin-top: 16px;
    flex: none;
  }
  .s-row {
    display: grid;
    grid-template-columns: 14px 1fr auto;
    gap: 14px;
    align-items: center;
    padding: 11px 0;
    border-bottom: 1px solid var(--app-border);
    font-size: var(--text-base);
  }
  .s-row:first-child {
    border-top: 1px solid var(--app-border);
  }
  .s-row .tk {
    color: var(--app-text-subtle);
  }
  .s-row .tk.on {
    color: var(--app-accent);
  }
  .s-row .k {
    color: var(--app-text-muted);
  }
  .s-row .v {
    display: inline-flex;
    align-items: center;
    gap: 14px;
    color: var(--app-text-subtle);
    font-size: var(--text-sm);
  }

  /* the one question on screen */
  .moment {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
  }
  .moment .ob-lead {
    margin-top: 20px;
  }
  .moment .ob-acts {
    margin-top: 30px;
  }
  .moment .tail {
    margin-top: 26px;
  }

  /* level meter — functional feedback, the one motion-free proof this screen has */
  .level {
    display: flex;
    align-items: center;
    gap: 24px;
    margin-top: 30px;
  }
  .level > div {
    max-width: 52ch;
  }
  .meter {
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 44px;
  }
  /* At rest the meter collapses to its own floor line — a 44px box holding 6px
     bars reads as debris, not as a quiet meter. */
  .meter:not(.lit) {
    height: 10px;
  }
  .meter i {
    width: 3px;
    display: block;
    border-radius: 1px;
    background: var(--app-border-hover);
  }
  .meter.lit i {
    background: var(--app-text-subtle);
  }

  .trail {
    display: flex;
    align-items: center;
    gap: 22px;
  }
  .upcoming {
    display: flex;
    gap: 26px;
    color: var(--app-text-subtle);
    font-size: var(--text-sm);
  }
  .faint {
    color: var(--app-text-subtle);
    opacity: 0.7;
  }
</style>
