<!--
  Settings → Intelligence → Speakers: the account-owner enrollment surface.

  Three jobs, all of them the user's: record (or re-record) a voiceprint at any
  time — including mid-recording, because the live-session guard around the
  bounded recorder is backend-owned; delete it, because storing a biometric
  sample with no delete affordance is not acceptable; and read, in plain words,
  whether recognition is actually on.

  It re-judges nothing. `enroll_account_owner_voice` returns either a stored
  profile or one of three typed rejections, and this file renders whichever
  arrived. It also does not flip `recognize_saved_people` — the backend does
  that as part of enrolling; the draft mirror below only keeps Settings' own
  autosave from writing the stale `false` back over it.
-->
<script lang="ts">
  import { setSettingsSection } from "$lib/settings/state/settings-find.svelte";

  // Every SettingRow below belongs to this section (⌘F row index scope, G7).
  setSettingsSection("speakers");

  import { onDestroy } from "svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import { VoiceEnrollmentStore } from "$lib/voice-enrollment.svelte";
  import {
    ENROLLMENT_CLIP_MS,
    ENROLLMENT_SENTENCE,
    recognitionReadout,
    rejectionMessage,
  } from "$lib/voice-enrollment";

  const c = getSettingsController();
  const rec = c.rec;

  const voice = new VoiceEnrollmentStore();
  void voice.load();
  onDestroy(() => voice.dispose());

  const enrolled = $derived(voice.owner !== null);
  const readout = $derived(
    recognitionReadout({
      enrolled,
      displayName: voice.owner?.displayName ?? null,
      separateSpeakers: rec.draftSpeakerSeparateSpeakers,
      recognizeSavedPeople: rec.draftSpeakerRecognizeSavedPeople,
      autoLabelOwner: rec.draftSpeakerAutoLabelOwner,
    }),
  );
  const clipSeconds = Math.round(ENROLLMENT_CLIP_MS / 1000);
  // Percent-of-full for the level meter. The take is 15 s of continuous speech,
  // so a coarse bar is all the feedback that is useful — it answers "is anything
  // arriving", not "what is my dBFS".
  const levelPercent = $derived(Math.round(voice.level * 100));
  const playbackSrc = $derived(voice.clipPath ? convertFileSrc(voice.clipPath) : null);

  let playbackFailed = $state(false);
  let playing = $state(false);
  let playbackEl = $state<HTMLAudioElement | null>(null);

  function togglePlayback() {
    if (!playbackEl) return;
    if (playing) {
      playbackEl.pause();
      playbackEl.currentTime = 0;
    } else {
      void playbackEl.play().catch(() => (playbackFailed = true));
    }
  }

  async function enroll() {
    const stored = await voice.enroll();
    // Enrolling turned `recognize_saved_people` on backend-side; mirror it into
    // the draft so the next autosave does not write the stale value back.
    if (stored) rec.draftSpeakerRecognizeSavedPeople = true;
  }

  async function removeVoiceprint() {
    const name = voice.owner?.displayName ?? "you";
    const ok = await confirm(
      `Delete the voiceprint saved for ${name}? Mnema stops recognising your voice and labels your turns “Speaker 1” again. Your recordings and transcripts are not touched.`,
      { title: "Delete voiceprint", kind: "warning", okLabel: "Delete", cancelLabel: "Keep" },
    );
    if (ok) await voice.deleteProfile();
  }

  function startTake() {
    playbackFailed = false;
    void voice.record();
  }
</script>

<SettingGroup
  title="Your voice"
  hint="Record a short sample so Mnema can label your turns with your name instead of “Speaker 1”."
>
  <SettingRow label="Voiceprint" full>
    {#snippet control()}
      <div class="voice-stack">
        <p class="group-hint voice-readout" aria-live="polite">
          {voice.loading ? "Checking for a saved voiceprint…" : readout}
        </p>

        {#if voice.justEnrolled}
          <p class="group-hint">Saved. Recognising saved people was switched on for you.</p>
        {/if}

        <p class="voice-sentence">{ENROLLMENT_SENTENCE}</p>

        {#if voice.stage === "recording"}
          <div class="voice-meter" aria-live="polite">
            <div class="voice-meter__track">
              <span class="voice-meter__fill" style={`width:${levelPercent}%`}></span>
            </div>
            <span class="voice-meter__count">
              Recording · {voice.secondsLeft}s left
            </span>
          </div>
        {:else if voice.stage === "review" && playbackSrc}
          <!-- A hidden <audio> plus one of the shell's own buttons, the same
               shape the dashboard's clip transport takes: the native controls
               render as a light grey pill that does not belong in this UI. -->
          <div class="voice-playback">
            <!-- svelte-ignore a11y_media_has_caption -->
            <audio
              bind:this={playbackEl}
              src={playbackSrc}
              onplay={() => (playing = true)}
              onpause={() => (playing = false)}
              onended={() => (playing = false)}
              onerror={() => (playbackFailed = true)}
              style="display:none"
            ></audio>
            <button
              type="button"
              class="btn btn--ghost"
              onclick={togglePlayback}
              disabled={playbackFailed}
            >
              {playing ? "Stop" : "Play the take"}
            </button>
            <p class="group-hint">
              {playbackFailed
                ? "Playback is unavailable, but the take was recorded — use it or record again."
                : "Listen back before you save it."}
            </p>
          </div>
        {/if}

        {#if voice.rejection}
          <p class="group-hint group-hint--warn">{rejectionMessage(voice.rejection)}</p>
        {/if}
        {#if voice.error}
          <p class="group-hint group-hint--warn">{voice.error}</p>
        {/if}

        <div class="voice-actions">
          {#if voice.stage === "review"}
            <button type="button" class="btn btn--primary" onclick={enroll}>Use this take</button>
            <button type="button" class="btn btn--ghost" onclick={() => voice.discard()}>Discard</button>
          {:else}
            <button
              type="button"
              class="btn btn--primary"
              onclick={startTake}
              disabled={voice.stage !== "idle"}
              aria-busy={voice.stage !== "idle"}
            >
              {#if voice.stage === "recording"}
                <ButtonSpinner />Recording
              {:else if voice.stage === "enrolling"}
                <ButtonSpinner />Checking the take
              {:else if enrolled}
                Record again ({clipSeconds}s)
              {:else}
                Record {clipSeconds} seconds
              {/if}
            </button>
            {#if enrolled}
              <button
                type="button"
                class="btn btn--danger"
                onclick={removeVoiceprint}
                disabled={voice.deleting || voice.stage !== "idle"}
                aria-busy={voice.deleting}
              >
                {#if voice.deleting}<ButtonSpinner />Deleting{:else}Delete voiceprint{/if}
              </button>
            {/if}
          {/if}
        </div>

        <p class="group-hint">
          The voiceprint never leaves this device. Recognition is imperfect and will not label every
          turn.
        </p>
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Label my voice automatically"
    description="When Mnema is confident a voice is yours, it applies your name without asking. Turn this off and it suggests your name and waits for you to confirm. Automatic labels are marked as automatic and can be undone. Only your own voice is ever labelled this way."
    divider={true}
  >
    {#snippet control()}
      <Switch
        bind:checked={rec.draftSpeakerAutoLabelOwner}
        ariaLabel="Label my voice automatically"
      />
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  .voice-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }

  /* The read-out is the row's headline answer, so it carries the normal text
     weight rather than the muted hint colour the other lines use. */
  .voice-readout {
    color: var(--app-text-strong);
  }

  /* The supplied sentence reads as a quotation to be performed, not as UI
     copy — hence the rule and the roomier measure. */
  .voice-sentence {
    margin: 0;
    padding: 10px 12px;
    border-left: 2px solid var(--app-border);
    font-size: var(--t-ui);
    line-height: 1.5;
    color: var(--app-text);
  }

  .voice-meter {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .voice-meter__track {
    flex: 1 1 auto;
    height: 8px;
    border-radius: 999px;
    background: var(--app-border);
    overflow: hidden;
  }

  .voice-meter__fill {
    display: block;
    height: 100%;
    background: var(--app-accent, currentColor);
    transition: width 120ms linear;
  }

  .voice-meter__count {
    flex-shrink: 0;
    font-family: var(--app-font-mono);
    font-size: 11px;
    color: var(--app-text-muted);
  }

  .voice-playback {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
  }

  .voice-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
</style>
