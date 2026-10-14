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
                    requestGeckoAccess(), recheckGeckoAccess()
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
      Gate any permission on a denial — Screen Recording included. macOS never
      re-prompts after one, so a hard gate would trap the user with no in-app
      recovery; a screen denial leaves an audio-only (or idle) session, which the
      Finale names. Never show system audio as confirmed: its grant
      cannot be read (ADR 0052), so it gets Request / Request again and a plain
      statement that macOS will not confirm it.
    holds
      One thing only: a Screen Recording grant made in THIS process. macOS hands
      ScreenCaptureKit the new grant only to a fresh one, so Continue becomes
      Relaunch and the flow resumes here (`RESUME_KEY` in onboarding-flow).

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
  import { openPermissionHelper, type PermissionHelperKind } from "$lib/shell/permission-helper";
  import { serializeError } from "../onboarding-mapping";
  import IconArrow from "~icons/lucide/arrow-right";
  import IconCheck from "~icons/lucide/check";
  import IconExternal from "~icons/lucide/external-link";
  import IconHalf from "~icons/lucide/circle-dashed";
  import IconInfo from "~icons/lucide/info";
  import IconRelaunch from "~icons/lucide/rotate-ccw";
  import IconRing from "~icons/lucide/circle";

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

  // The one hold on this screen: a screen grant made in THIS process reads
  // granted while ScreenCaptureKit still cannot use it, so continuing would only
  // reach the Finale to fail there. A denial holds nothing.
  const gate = $derived(screenGate(value("screen"), flow.screenNeedsRelaunch));

  // Live levels from the system-audio probe (`start_system_audio_level_probe`).
  // The permission state cannot stand in for these: `assumed_working` is sticky
  // cross-session evidence, so it claims sound on a silent Mac and stays quiet on
  // a loud one until a real recording runs — both of the reported bugs.
  let levels = $state<number[]>(Array(METER_BARS).fill(0));
  let probing = $state(false);
  const soundArriving = $derived(levels.some((level) => level > 0));
  /** A previous session's tap heard sound. Evidence, not a live reading. */
  const heardBefore = $derived(value("systemAudio") === "assumed_working");
  /** Persisted evidence that a past tap heard nothing while something played. */
  const sysBlocked = $derived(value("systemAudio") === "possibly_blocked");
  // ON3: every "Open System Settings" opens the pane AND floats the helper
  // window; a failure lands in the screen's error line.
  async function openHelper(kind: PermissionHelperKind): Promise<void> {
    c.errorMessage = null;
    try {
      await openPermissionHelper(kind);
    } catch (err) {
      c.errorMessage = serializeError(err);
    }
  }

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
    if (!gate.ready) {
      void invoke("request_app_relaunch");
      return;
    }
    if (isLast) {
      onContinue();
      return;
    }
    if (current !== null) passed = [...passed, current];
  }

  const primaryLabel = $derived.by(() => {
    if (!gate.ready) return gate.primaryLabel;
    if (isLast) return "Capture & Storage";
    if (current === "screen") return gate.primaryLabel;
    if (current === "microphone" && isDenied(value("microphone"))) {
      return "Continue without the microphone";
    }
    return "Continue";
  });
</script>

<!-- Coming back from System Settings: pick up a grant without Re-check. -->
<svelte:window onfocus={() => void recheck()} />

<div class="ob-main">
  <div class="ob-col">
    <span class="mx-kicker">One at a time</span>

    {#if flow.screenNeedsRelaunch}
      <div class="mx-notice" data-tone="info">
        <span class="mx-notice__icon"><IconRelaunch width="15" height="15" /></span>
        <b class="mx-notice__title">Screen recording needs one relaunch</b>
        <p class="mx-notice__text">macOS gives that grant to a fresh copy of Mnema. Setup resumes right here.</p>
      </div>
    {/if}

    {#if settled.length > 0}
      <ul class="ob-ledger">
        {#each settled as id (id)}
          {@const half = (id === "systemAudio" && soundArriving) || (id === "screen" && flow.screenNeedsRelaunch)}
          <li class:warn={(id !== "accessibility" && isDenied(value(id))) || (id === "systemAudio" && sysBlocked)}>
            {#if half}
              <IconHalf class="ob-mark" />
            {:else if ledgerGranted(id)}
              <IconCheck class="ob-mark ob-mark--on" />
            {:else}
              <IconRing class="ob-mark" />
            {/if}
            <span>{NAMES[id]}</span>
            <span>{ledgerValue(id)}</span>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="ob-moment">
      {#if current === "screen"}
        {#if isDenied(value("screen"))}
          <h1 class="mx-display">Screen recording</h1>
          <p class="mx-prose">
            You said no — and this is the one Mnema cannot run without. macOS will not ask again, so the
            switch has to be flipped in System Settings.
          </p>
          <div class="ob-acts">
            <button class="mx-btn" onclick={() => openHelper("screen")}>Open System Settings<IconExternal width="12" height="12" /></button>
            <button class="mx-btn" disabled={c.refreshingPerms} onclick={recheck}>
              {c.refreshingPerms ? "Checking…" : "Re-check"}
            </button>
            <span class="ob-fine">Privacy &amp; Security → Screen Recording → Mnema</span>
          </div>
          <p class="ob-fine">Flip it, come back, press Re-check — no need to quit Mnema yourself.</p>
        {:else}
          <h1 class="mx-display">Your screen.</h1>
          <p class="mx-prose">The one Mnema is built on — it records the screen so you can scrub back to it.</p>
          <div class="ob-acts">
            <button class="mx-btn" disabled={c.requestingPerm !== null} onclick={() => request("screen")}>
              {c.requestingPerm === "screen" ? "Requesting…" : "Request access"}
            </button>
            <span class="ob-fine">macOS raises its own prompt. Nothing is recorded until you finish setup.</span>
          </div>
        {/if}
      {:else if current === "microphone"}
        {#if isDenied(value("microphone"))}
          <h1 class="mx-display">Microphone</h1>
          <p class="mx-prose">You said no. Mnema keeps working — without transcripts or speaker names.</p>
          <div class="ob-acts">
            <button class="mx-btn" onclick={() => openHelper("microphone")}>Open System Settings<IconExternal width="12" height="12" /></button>
            <button class="mx-btn" disabled={c.refreshingPerms} onclick={recheck}>
              {c.refreshingPerms ? "Checking…" : "Re-check"}
            </button>
            <span class="ob-fine">Privacy &amp; Security → Microphone → Mnema</span>
          </div>
          <p class="ob-fine">macOS never asks again on its own, which is why this screen never blocks.</p>
        {:else}
          <h1 class="mx-display">Your microphone.</h1>
          <p class="mx-prose">Only what you say, transcribed on this Mac — no audio ever leaves it.</p>
          <div class="ob-acts">
            <button class="mx-btn" disabled={c.requestingPerm !== null} onclick={() => request("microphone")}>
              {c.requestingPerm === "microphone" ? "Requesting…" : "Request access"}
            </button>
            <span class="ob-fine">Off by itself, this only costs you transcripts and speaker names.</span>
          </div>
        {/if}
      {:else if current === "systemAudio"}
        <h1 class="mx-display">Play anything.</h1>
        <p class="mx-prose">macOS will not tell us whether you granted this — sound arriving is the only proof.</p>
        <div class="ob-level">
          <!-- Every bar is one poll of a real tap: the meter moves with what the
               tap is delivering and flatlines the moment it stops. Square-rooted
               because the level is a linear peak. -->
          <div class="ob-meter" class:live={probing || soundArriving} class:lit={soundArriving} aria-hidden="true">
            {#each levels as level, i (i)}
              <i style="height:{4 + Math.round(Math.sqrt(level) * 30)}px"></i>
            {/each}
          </div>
          <div class="ob-level__state">
            <b>
              {#if soundArriving}
                Sound is arriving.
              {:else if probing}
                Listening…
              {:else if c.sysAudioPromptRaised}
                Requested · unconfirmed
              {:else}
                Nothing has arrived yet
              {/if}
            </b>
            {#if !soundArriving}
              <span class="ob-fine">
                {probing ? "Play something — a video, a song. " : ""}Silent is not denied — it might just be
                a quiet Mac.
              </span>
            {/if}
          </div>
        </div>
        {#if sysBlocked}
          <div class="mx-notice" data-tone="info">
            <span class="mx-notice__icon"><IconInfo width="15" height="15" /></span>
            <b class="mx-notice__title">System audio may be blocked</b>
            <p class="mx-notice__text">
              A minute of silence while something played. Check the switch, or carry on — the mic still records.
            </p>
            <div class="mx-notice__acts">
              <button class="mx-btn mx-btn--sm" onclick={() => openHelper("systemAudio")}>
                Open System Settings<IconExternal width="12" height="12" />
              </button>
            </div>
          </div>
        {/if}
        <div class="ob-acts">
          <button class="mx-btn" disabled={c.requestingPerm !== null} onclick={() => request("systemAudio")}>
            {#if c.requestingPerm === "systemAudio"}
              Requesting…
            {:else}
              {c.sysAudioPromptRaised ? "Request again" : "Request"}
            {/if}
          </button>
          <span class="ob-fine">A prompt you closed by accident looks exactly like a No.</span>
        </div>
      {:else if current === "accessibility"}
        <h1 class="mx-display">Browser page addresses.</h1>
        <p class="mx-prose">
          Optional, and only for {c.geckoInstalledNames.join(" and ") || "Firefox and Zen"} — every other
          browser tells Mnema its page address already.
        </p>
        {#if accessibilityAsked}
          <div class="ob-acts">
            <button class="mx-btn" onclick={() => openHelper("accessibility")}>Open System Settings<IconExternal width="12" height="12" /></button>
            <button class="mx-btn" disabled={c.recheckingGeckoAccess} onclick={() => c.recheckGeckoAccess()}>
              {c.recheckingGeckoAccess ? "Checking…" : "Re-check"}
            </button>
            <span class="ob-fine">Privacy &amp; Security → Accessibility → Mnema</span>
          </div>
        {:else}
          <div class="ob-acts">
            <button class="mx-btn" disabled={c.requestingGeckoAccess} onclick={askAccessibility}>
              {c.requestingGeckoAccess ? "Requesting…" : "Request access"}
            </button>
            <span class="ob-fine">Privacy &amp; Security → Accessibility → Mnema</span>
          </div>
        {/if}
        <p class="ob-fine">The broadest grant there is, so it never rides along with the others.</p>
      {:else}
        <h1 class="mx-display">That is everything.</h1>
        <p class="mx-prose">Nothing left to ask — all of it stays changeable in Settings.</p>
      {/if}
    </div>
  </div>
</div>

<footer class="ob-foot">
  <p class="ob-foot__note">
    {#each upcoming as id, i (id)}{i === 0 ? "next" : "   then"} · <b>{NAMES[id]}</b
      >{#if id === "accessibility"}&nbsp;(optional){/if}{/each}
  </p>
  <button class="mx-btn mx-btn--ghost" onclick={onBack}>Back</button>
  <button class="mx-btn mx-btn--primary" onclick={advance}>
    {primaryLabel}<IconArrow class="ob-btn-arrow" width="14" height="14" />
  </button>
</footer>

<style>
  .ob-ledger {
    display: grid;
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--mx-hairline);
    max-width: 520px;
  }
  .ob-ledger li {
    display: grid;
    grid-template-columns: 14px 160px 1fr;
    align-items: center;
    gap: var(--s-3);
    height: 36px;
    border-bottom: 1px solid var(--mx-hairline);
    font: 500 var(--text-md) / 1 var(--font-sans);
    color: var(--app-text);
  }
  .ob-ledger li span:last-child {
    font: 400 var(--text-base) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .ob-ledger li.warn span:last-child {
    color: var(--app-warn);
  }
  .ob-moment {
    display: grid;
    gap: var(--s-4);
    justify-items: start;
  }
  .ob-moment .mx-display {
    font-size: 36px;
  }
  .ob-moment .mx-notice {
    max-width: 640px;
    justify-self: stretch;
  }
  .ob-col > .mx-notice {
    max-width: 640px;
  }

  /* level meter — the only proof this screen can offer, so it moves with the tap */
  .ob-level {
    display: flex;
    align-items: center;
    gap: var(--s-5);
    padding: var(--s-3) var(--s-4);
    border-radius: var(--r-lg);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
  }
  .ob-level__state {
    display: grid;
    gap: 4px;
  }
  .ob-level__state b {
    font: 600 var(--text-md) / 1.2 var(--font-sans);
    color: var(--app-text-strong);
  }
  .ob-meter {
    display: flex;
    align-items: center;
    gap: 3px;
    height: 40px;
  }
  .ob-meter i {
    width: 4px;
    border-radius: 2px;
    background: var(--chart-grey-2);
    transition: height 120ms var(--ease-quart);
  }
  .ob-meter.live i {
    background: var(--chart-grey-4);
  }
  .ob-meter.lit i {
    background: var(--app-source-sysaudio);
  }
  @media (prefers-reduced-motion: reduce) {
    .ob-meter i {
      transition: none;
    }
  }
</style>
