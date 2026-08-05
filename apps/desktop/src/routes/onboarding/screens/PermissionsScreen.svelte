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
      Gate the microphone, system audio or Accessibility — macOS never re-prompts
      after a denial, so a hard gate on an OPTIONAL source would trap the user
      with no in-app recovery. Never show system audio as confirmed: its grant
      cannot be read (ADR 0052), so it gets Request / Request again and a plain
      statement that macOS will not confirm it.
    gates
      Screen Recording, and only it. Mnema records the screen — without that
      grant there is nothing to record — so Continue stays blocked until macOS
      reports it granted. The gate is passable without leaving onboarding:
      Request → (on a denial) deep-link to the right pane → Re-check. A grant
      made in THIS process is not enough: macOS hands ScreenCaptureKit the new
      grant only to a fresh one, so Continue becomes Relaunch and the flow
      resumes here (`RESUME_KEY` in onboarding-flow).

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
  import { isDenied, isGranted, screenGate } from "$lib/onboarding/permissions-gate";

  let {
    flow,
    onContinue,
    onBack,
  }: { flow: OnboardingFlow; onContinue: () => void; onBack: () => void } = $props();

  type MomentId = PermissionKey | "accessibility";

  /** One bar per poll, newest on the right — ~2.5 s of sound at a glance. */
  const METER_BARS = 20;
  const METER_POLL_MS = 120;

  const NAMES: Record<MomentId, string> = {
    screen: "Screen recording",
    microphone: "Microphone",
    systemAudio: "System audio",
    accessibility: "Accessibility",
  };

  const c = $derived(flow.controller);
  const value = (key: PermissionKey): PermissionValue | undefined => c.permissions?.[key];


  /** Moments the user has continued past. A denial can never be un-denied here. */
  let passed = $state<MomentId[]>([]);
  /** Screen Recording granted *in this process* — macOS only honours it after a relaunch. */

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

  // The one required grant. `flow.screenNeedsRelaunch` blocks just as hard as a
  // missing grant: the permission reads granted while ScreenCaptureKit in THIS
  // process still cannot use it, so continuing would only reach the Finale to
  // fail there.
  const screenReady = $derived(screenGate(value("screen"), flow.screenNeedsRelaunch).ready);

  // Live levels from the system-audio probe (`start_system_audio_level_probe`).
  // The permission state cannot stand in for these: `assumed_working` is sticky
  // cross-session evidence, so it claims sound on a silent Mac and stays quiet on
  // a loud one until a real recording runs — both of the reported bugs.
  let levels = $state<number[]>(Array(METER_BARS).fill(0));
  let probing = $state(false);
  const soundArriving = $derived(levels.some((level) => level > 0));
  /** A previous session's tap heard sound. Evidence, not a live reading. */
  const heardBefore = $derived(value("systemAudio") === "assumed_working");

  // Listen only once the prompt has been raised (or a past session proved the
  // grant): building the tap IS the macOS prompt, and it must stay behind the
  // Request button. The probe self-stops seconds after polling ends, so leaving
  // this screen — or closing the window — needs no teardown of its own.
  $effect(() => {
    if (current !== "systemAudio" || !(c.sysAudioPromptRaised || heardBefore)) return;
    let live = true;
    void invoke("start_system_audio_level_probe").catch(() => {});
    const timer = setInterval(async () => {
      const level = await invoke<number | null>("get_system_audio_probe_level").catch(() => null);
      if (!live) return;
      probing = level !== null;
      levels = [...levels.slice(1), level ?? 0];
    }, METER_POLL_MS);
    return () => {
      live = false;
      clearInterval(timer);
      levels = Array(METER_BARS).fill(0);
      probing = false;
    };
  });

  function ledgerValue(id: MomentId): string {
    if (id === "accessibility") return c.geckoTrusted ? "granted" : "skipped";
    const v = value(id);
    if (id === "systemAudio") {
      if (soundArriving) return "sound is arriving";
      // Never "sound is arriving" for evidence: it is what a past session heard,
      // and reading it as the present is exactly the lie this screen cannot tell.
      if (heardBefore) return "sound heard earlier";
      return c.sysAudioPromptRaised ? "requested · unconfirmed" : "not requested";
    }
    if (isGranted(v)) {
      return id === "screen" && flow.screenNeedsRelaunch ? "granted · needs one relaunch" : "granted";
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
    if (key === "screen" && !before && isGranted(value("screen"))) flow.screenNeedsRelaunch = true;
  }

  // A grant made in System Settings lands the same in-process staleness as an
  // in-app one, so Re-check raises the same relaunch requirement.
  async function recheck(): Promise<void> {
    const before = isGranted(value("screen"));
    await c.refreshPermissions();
    if (!before && isGranted(value("screen"))) flow.screenNeedsRelaunch = true;
  }

  async function askAccessibility(): Promise<void> {
    accessibilityAsked = true;
    await c.requestGeckoAccess();
  }

  function advance(): void {
    if (flow.screenNeedsRelaunch) {
      void invoke("request_app_relaunch");
      return;
    }
    if (!screenReady) return;
    if (isLast) {
      onContinue();
      return;
    }
    if (current !== null) passed = [...passed, current];
  }

  const primaryLabel = $derived.by(() => {
    if (flow.screenNeedsRelaunch) return "Relaunch to continue";
    if (!screenReady) return "Screen recording required";
    if (isLast) return "Capture & Storage";
    // Screen can no longer reach this branch — it is the gate, not a choice.
    if (current !== null && current !== "accessibility" && isDenied(value(current))) {
      return "Continue without the microphone";
    }
    return "Continue";
  });
</script>

<span class="ob-m">One at a time · only the screen is required</span>

{#if settled.length > 0}
  <div class="settled">
    {#each settled as id (id)}
      <div class="s-row">
        <span class="tk" class:on={ledgerGranted(id)} aria-hidden="true">{ledgerGranted(id) ? "✓" : "○"}</span>
        <span class="k">{NAMES[id]}</span>
        <!-- The relaunch itself is the primary action below, not a second
             button here: with Screen Recording required, nothing continues
             until it happens. -->
        <span class="v">{ledgerValue(id)}</span>
      </div>
    {/each}
  </div>
{/if}

<div class="moment">
  {#if current === "screen"}
    {#if isDenied(value("screen"))}
      <h1 class="ob-disp mid">Screen recording</h1>
      <p class="ob-lead">
        You said no — and this is the one Mnema cannot run without. macOS will not ask again, so the
        switch has to be flipped in System Settings.
      </p>
      <div class="ob-acts">
        <button class="ob-btn" onclick={() => request("screen")}>Open System Settings&nbsp; ›</button>
        <button class="ob-btn" disabled={c.refreshingPerms} onclick={recheck}>
          {c.refreshingPerms ? "Checking…" : "Re-check"}
        </button>
        <span class="ob-fine">Privacy &amp; Security → Screen Recording → Mnema</span>
      </div>
      <p class="ob-fine tail">Flip it, come back, press Re-check — no need to quit Mnema yourself.</p>
    {:else}
      <h1 class="ob-disp mid">Your screen.</h1>
      <p class="ob-lead">
        The one Mnema is built on — it records the screen so you can scrub back to it. This is the
        only grant setup will not continue without.
      </p>
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
        <button class="ob-btn" disabled={c.refreshingPerms} onclick={recheck}>
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
      <!-- Every bar is one poll of a real tap: the meter moves with what the tap
           is delivering and flatlines the moment it stops. Square-rooted because
           the level is a linear peak — untouched, ordinary playback barely lifts
           off the floor. -->
      <div class="meter" class:live={probing || soundArriving} class:lit={soundArriving} aria-hidden="true">
        {#each levels as level, i (i)}
          <i style="height:{6 + Math.round(Math.sqrt(level) * 30)}px"></i>
        {/each}
      </div>
      <div>
        <div class="ob-strong">
          {#if soundArriving}
            Sound is arriving.
          {:else if probing}
            Listening…
          {:else if c.sysAudioPromptRaised}
            Requested · unconfirmed
          {:else}
            Nothing has arrived yet
          {/if}
        </div>
        {#if !soundArriving}
          <p class="ob-fine">
            {probing ? "Play something — a video, a song. " : ""}Silent is not denied — it might just
            be a quiet Mac.
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
      {#if flow.screenNeedsRelaunch}
        <span class="ob-fine">
          macOS gives that grant to a fresh copy of Mnema. Setup resumes right here.
        </span>
      {:else if upcoming.length > 0}
        <span class="upcoming">
          {#each upcoming as id, i (id)}
            <span>
              {i === 0 ? "next" : "then"} · {NAMES[id]}
              {#if id === "accessibility"}<span class="faint">(optional)</span>{/if}
            </span>
          {/each}
        </span>
      {/if}
    </div>
    <button
      class="ob-btn primary"
      disabled={!screenReady && !flow.screenNeedsRelaunch}
      onclick={advance}
    >
      {primaryLabel}&nbsp; →
    </button>
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
    font-size: var(--t-ui);
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
    font-size: var(--t-meta);
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

  /* level meter — the only proof this screen can offer, so it moves with the tap */
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
  /* Before anything is listening the meter collapses to its own floor line — a
     44px box holding 6px bars reads as debris, not as a quiet meter. */
  .meter:not(.live) {
    height: 10px;
  }
  .meter i {
    width: 3px;
    display: block;
    border-radius: 1px;
    background: var(--app-border-hover);
    transition: height 120ms linear;
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
    font-size: var(--t-meta);
  }
  .faint {
    color: var(--app-text-subtle);
    opacity: 0.7;
  }
</style>
