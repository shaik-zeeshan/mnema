<script lang="ts">
  // Windows-only GPU Acceleration panel (#137, Slice 5 / ADR 0005).
  //
  // Surfaces the NVIDIA CUDA Execution Backend's provisioning lifecycle —
  // detect → offer → downloading → installed/working/failing — plus the default-on
  // "Use GPU acceleration" (Force-CPU) override. It mirrors `Speakers.svelte`'s
  // structure and styling exactly: a `SettingGroup` + `SettingRow` frame, the shared
  // `.model-status` card, `.download-progress` bar, `.group-hint` copy, `.btn`
  // actions, and the `Switch` toggle — no new visual language, just the existing
  // settings primitives and `var(--app-*)` tokens.
  //
  // The state machine (read off `gpuAccelerationState` + the GPU pack status/progress
  // from Slice 4) is:
  //   not Windows        → render nothing (the whole group is `{#if isWindows}`)
  //   !gpuDetected       → informational "running on CPU; needs an NVIDIA GPU"
  //   downloadRunning    → progress bar + Cancel
  //   gpu, !packInstalled→ the opt-in offer: explain + NVIDIA license consent + Enable
  //   packInstalled      → installed/working: last-run mode + Remove + the toggle
  //   lastCudaFallbackReason present → a warn notice (THE "why isn't my GPU used?")
  //
  // Backend is orthogonal to identity: the toggle never changes WHO is recognized,
  // only WHETHER the (faster) GPU path is attempted — so it lives here, not as a
  // model/backend selector.

  import { ask } from "@tauri-apps/plugin-dialog";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import { detectKeyboardPlatform } from "$lib/keyboard";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import { formatBytes } from "$lib/settings/state/format";

  const c = getSettingsController();
  const models = c.models;

  // Windows-only: macOS is always CoreML (no pack, no toggle). Computed once — the
  // platform can't change mid-session. The rail hides the section too (filterPlatform),
  // so this guard and the rail guard are the two mirrored halves of the gate.
  const isWindows = detectKeyboardPlatform() === "windows";

  // ── Store-read aliases (mirrors Speakers.svelte's `$derived` selectors). ──────
  const state = $derived(models.gpuAccelerationState);
  const packStatus = $derived(models.gpuPackStatus);
  const progress = $derived(models.gpuPackDownloadProgress);
  const loadingGpuAccelerationState = $derived(models.loadingGpuAccelerationState);
  const loadingGpuPackStatus = $derived(models.loadingGpuPackStatus);
  const gpuAccelerationStateError = $derived(models.gpuAccelerationStateError);
  const gpuPackError = $derived(models.gpuPackError);
  const gpuPackDownloadError = $derived(models.gpuPackDownloadError);
  const gpuPackDeleteMessage = $derived(models.gpuPackDeleteMessage);
  const startingGpuPackDownload = $derived(models.startingGpuPackDownload);
  const cancellingGpuPackDownload = $derived(models.cancellingGpuPackDownload);
  const deletingGpuPack = $derived(models.deletingGpuPack);
  const settingUseGpuAcceleration = $derived(models.settingUseGpuAcceleration);

  // A download is in flight while either the start request is pending or the latest
  // phase (from the progress event, falling back to the status snapshot) is a
  // non-terminal one. Mirrors Speakers' `selectedSpeakerDownloadRunning`.
  const downloadPhase = $derived(progress?.status ?? packStatus?.downloadState ?? null);
  const downloadRunning = $derived(
    startingGpuPackDownload ||
      downloadPhase === "starting" ||
      downloadPhase === "downloading" ||
      downloadPhase === "installing",
  );
  // Percent from byte counts (the pack progress has no precomputed percent). Null
  // until a total is known, so the bar falls back to a small indeterminate width.
  const downloadPercent = $derived.by(() => {
    const total = progress?.totalBytes ?? null;
    if (!total || total <= 0) return null;
    return Math.min(100, Math.round(((progress?.downloadedBytes ?? 0) / total) * 100));
  });

  // Human label for the last job's backend. Null when no job has run yet.
  function lastRunLabel(mode: string | null): string | null {
    if (!mode) return null;
    switch (mode) {
      case "cuda":
        return "Last run: GPU (CUDA)";
      case "cpu":
        return "Last run: CPU";
      case "coreml":
        return "Last run: GPU (CoreML)";
      default:
        return `Last run: ${mode}`;
    }
  }

  // ── Actions ───────────────────────────────────────────────────────────────
  const refresh = () => {
    void c.loadGpuAccelerationState();
    void c.loadGpuPackStatus();
  };

  // The NVIDIA redist is fetched under terms the user must accept in-app. The two
  // license URLs are shown in the offer; this is the explicit consent gate before a
  // single byte is fetched — a plugin-dialog confirm (AGENTS.md: never window.confirm).
  async function enableGpuAcceleration() {
    const accepted = await ask(
      "The GPU Acceleration Pack downloads NVIDIA CUDA and cuDNN redistributables directly from NVIDIA. Continuing means you accept NVIDIA's CUDA and cuDNN license agreements (linked in Settings).",
      {
        title: "Accept NVIDIA licenses",
        kind: "info",
        okLabel: "Accept & download",
        cancelLabel: "Cancel",
      },
    );
    if (!accepted) return;
    await c.startGpuPackDownload(true);
  }

  const cancelGpuPackDownload = () => c.cancelGpuPackDownload();
  const deleteGpuPack = () => c.deleteGpuPack();
  const setUseGpu = (value: boolean) => c.setUseGpuAcceleration(value);
</script>

{#if isWindows}
  <SettingGroup
    id="settings-section-gpuAcceleration"
    title="GPU acceleration"
    hint="Speaker analysis can run on an NVIDIA GPU via an opt-in CUDA pack. Windows only — macOS is unaffected."
  >
    {#snippet actions()}
      <ReloadButton
        onclick={refresh}
        busy={loadingGpuAccelerationState || loadingGpuPackStatus}
        title="Refresh"
        label="Refresh GPU acceleration status"
      />
    {/snippet}

    <SettingRow
      label="Execution backend"
      description="Diarization and recognition produce the same speakers on CPU or GPU — the GPU only makes them faster. Identity is never affected."
      full
      divider={false}
    >
      {#snippet control()}
        <!-- The state machine stacks the status card / offer / progress / actions in
             one bordered sub-block, exactly like Speakers' `.speaker-stack`. -->
        <div class="gpu-stack">
          {#if gpuAccelerationStateError}
            <p class="group-hint group-hint--warn">Failed to load GPU acceleration status: {gpuAccelerationStateError}</p>
          {:else if !state}
            {#if loadingGpuAccelerationState}
              <p class="group-hint">Checking GPU acceleration…</p>
            {:else}
              <p class="group-hint group-hint--warn">No GPU acceleration status is available.</p>
            {/if}
          {:else}
            <!-- `state` is non-null in this branch (the `!state` guard above failed),
                 so every `state.*` access below is safe — the same null-guard-then-
                 access shape Speakers.svelte uses for `selectedSpeakerModel`. -->
            {#if !state.gpuDetected}
              <!-- No NVIDIA GPU: CPU only, no offer. -->
              <div class="model-status">
                <div>
                  <div class="model-status__title">Running on CPU</div>
                  <div class="model-status__meta">no nvidia gpu detected</div>
                </div>
                <span class="model-status__pill">cpu</span>
              </div>
              <p class="group-hint">Speaker analysis runs on the CPU. GPU acceleration needs an NVIDIA GPU with a current driver; none was detected on this machine.</p>
            {:else if downloadRunning}
              <!-- Downloading the NVIDIA redist (mirrors Speakers' download-progress). -->
              <div class="download-progress" aria-live="polite">
                <div class="download-progress__bar">
                  <span style={`width: ${downloadPercent ?? 8}%`}></span>
                </div>
                <p class="group-hint">
                  {progress?.status ?? "downloading"}
                  {#if downloadPercent !== null} · {downloadPercent}%{/if}
                  {#if progress?.component} · {progress.component}{/if}
                  {#if progress?.message} · {progress.message}{/if}
                </p>
                <button class="btn btn--ghost" onclick={cancelGpuPackDownload} disabled={cancellingGpuPackDownload}>
                  {cancellingGpuPackDownload ? "Cancelling" : "Cancel download"}
                </button>
              </div>
            {:else if !state.packInstalled}
              <!-- GPU detected, pack not installed: the opt-in offer + license consent. -->
              <div class="model-status">
                <div>
                  <div class="model-status__title">NVIDIA GPU detected</div>
                  <div class="model-status__meta">gpu acceleration available</div>
                </div>
                <span class="model-status__pill">not installed</span>
              </div>
              <p class="group-hint">
                Install the GPU Acceleration Pack to run speaker analysis far faster on your NVIDIA GPU. It downloads the NVIDIA CUDA{packStatus ? ` ${packStatus.requiredCudaVersion}` : ""} and cuDNN{packStatus ? ` ${packStatus.requiredCudnnVersion}` : ""} redistributables{packStatus ? ` (${formatBytes(packStatus.totalBytes)})` : ""} into app-managed storage. Speaker analysis keeps working on the CPU until it finishes.
              </p>
              {#if packStatus}
                <p class="group-hint">
                  <strong>Licenses:</strong> NVIDIA CUDA — {packStatus.licenseUrls.cuda}; cuDNN — {packStatus.licenseUrls.cudnn}. Downloading means you accept NVIDIA's license agreements.
                </p>
              {/if}
              <div class="debug-log-actions">
                <button class="btn btn--ghost" onclick={enableGpuAcceleration} disabled={startingGpuPackDownload}>
                  {startingGpuPackDownload ? "Starting" : "Enable GPU acceleration"}
                </button>
              </div>
            {:else}
              <!-- Installed / working: status + last-run mode + the toggle + Remove. -->
              <div class="model-status model-status--available">
                <div>
                  <div class="model-status__title">GPU acceleration installed</div>
                  <div class="model-status__meta">{lastRunLabel(state.lastExecutionMode) ?? "nvidia cuda + cudnn ready"}</div>
                </div>
                <span class="model-status__pill">installed</span>
              </div>
              <Switch
                checked={state.useGpu}
                onCheckedChange={setUseGpu}
                disabled={settingUseGpuAcceleration}
                label="Use GPU acceleration"
                description="On by default. Turn off to force CPU on the next job without uninstalling the pack."
              />
              <div class="debug-log-actions">
                <button class="btn btn--danger" onclick={deleteGpuPack} disabled={deletingGpuPack || downloadRunning}>
                  {deletingGpuPack ? "Deleting" : "Remove GPU acceleration pack"}
                </button>
              </div>
            {/if}

            <!-- The single "why isn't my GPU used?" diagnostic: shown whenever the
                 last job fell back from CUDA, regardless of the branch above. Mirrors
                 how Speakers surfaces `failureMessage`. -->
            {#if state.lastCudaFallbackReason}
              <p class="group-hint group-hint--warn"><strong>GPU initialization failed — ran on CPU:</strong> {state.lastCudaFallbackReason}</p>
            {/if}
          {/if}

          {#if gpuPackError}
            <p class="group-hint group-hint--warn">Failed to load GPU pack status: {gpuPackError}</p>
          {/if}
          {#if gpuPackDownloadError}
            <p class="group-hint group-hint--warn">GPU acceleration action failed: {gpuPackDownloadError}</p>
          {/if}
          {#if gpuPackDeleteMessage}
            <p class="group-hint">{gpuPackDeleteMessage}</p>
          {/if}
        </div>
      {/snippet}
    </SettingRow>
  </SettingGroup>
{/if}

<style>
  /* The backend row stacks the status card / offer / progress / toggle into one
     bordered sub-block; primitives only gap whole rows. Identical to Speakers'
     `.speaker-stack` so the two panels read the same. */
  .gpu-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }
</style>
