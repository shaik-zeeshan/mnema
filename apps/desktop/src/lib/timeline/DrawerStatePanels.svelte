<script lang="ts">
  // Frame 6: every non-happy path the drawer can reach, each with a headline, a
  // plain-language sentence, the action that leads out, and the provenance the
  // app already parses. These used to be bare <p>s.
  import DrawerNotice from "./DrawerNotice.svelte";
  import type { DrawerPanelKind } from "./audio-drawer-view";

  interface Props {
    panel: DrawerPanelKind;
    durationLabel: string;
    /** `stage 2 of 3 · queued 40s ago`. */
    processingFootnote: string;
    /** `skipReason silent · audioPeak 0.004` / `chunkingMode safe_chunked · 5 chunks`. */
    provenanceFootnote: string;
    noSpeechNotice: string;
    transcriptError: string | null;
    speakerTurnsError: string | null;
    rerunDisabled: boolean;
    rerunLoading: boolean;
    speakerRetryDisabled: boolean;
    speakerRetryLoading: boolean;
    onPlayAnyway: () => void;
    onRerun: () => void;
    onRetrySpeakers: () => void;
    onReadWithoutSpeakers: () => void;
  }

  let {
    panel,
    durationLabel,
    processingFootnote,
    provenanceFootnote,
    noSpeechNotice,
    transcriptError,
    speakerTurnsError,
    rerunDisabled,
    rerunLoading,
    speakerRetryDisabled,
    speakerRetryLoading,
    onPlayAnyway,
    onRerun,
    onRetrySpeakers,
    onReadWithoutSpeakers,
  }: Props = $props();
</script>

{#if panel === "skeleton"}
  <DrawerNotice kind="skeleton" />
{:else if panel === "processing"}
  <DrawerNotice
    title="Still working on this one"
    body="Speech detection runs first, then transcription, then speaker analysis. The text appears as each stage lands — you don't have to wait here."
    footnote={processingFootnote}
  />
{:else if panel === "no-speech"}
  <DrawerNotice
    title="No speech in this segment"
    body={`${durationLabel} of room tone. Nothing was transcribed because there was nothing to transcribe.`}
    footnote={provenanceFootnote || noSpeechNotice}
  >
    {#snippet actions()}
      <button type="button" class="btn" onclick={onPlayAnyway}>Play it anyway</button>
      <button type="button" class="btn" disabled={rerunDisabled} onclick={onRerun}
        >Rerun analysis</button
      >
    {/snippet}
  </DrawerNotice>
{:else if panel === "failed"}
  <DrawerNotice
    title="This segment could not be transcribed"
    body={transcriptError ?? "The transcription job failed."}
    footnote={provenanceFootnote}
  >
    {#snippet actions()}
      <button type="button" class="btn btn--primary" disabled={rerunDisabled} onclick={onRerun}
        >{rerunLoading ? "Retrying…" : "Retry transcription"}</button
      >
    {/snippet}
  </DrawerNotice>
{:else if panel === "speakers-failed"}
  <DrawerNotice
    title="The words are here. The speakers aren't."
    body={speakerTurnsError ??
      "Transcription finished; speaker analysis stopped partway through. You can read the transcript now and retry the speaker pass."}
    footnote={provenanceFootnote}
  >
    {#snippet actions()}
      <button type="button" class="btn btn--primary" onclick={onReadWithoutSpeakers}
        >Read without speakers</button
      >
      <button
        type="button"
        class="btn"
        disabled={speakerRetryDisabled}
        onclick={onRetrySpeakers}
        >{speakerRetryLoading ? "Retrying…" : "Retry speaker analysis"}</button
      >
    {/snippet}
  </DrawerNotice>
{:else}
  <DrawerNotice
    title="Speaker analysis hasn't run for this segment"
    body="It was captured before this pass was turned on. Running it now costs about ten seconds and changes nothing else."
  >
    {#snippet actions()}
      <button type="button" class="btn btn--primary" disabled={rerunDisabled} onclick={onRerun}
        >{rerunLoading ? "Starting…" : "Run speaker analysis"}</button
      >
    {/snippet}
  </DrawerNotice>
{/if}

<!-- `.btn` / `.btn--primary` come from the global design system (+layout.svelte). -->
