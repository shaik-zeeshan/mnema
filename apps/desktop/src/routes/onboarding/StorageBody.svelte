<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import type { OnboardingController } from "./onboarding.svelte";
  import RetentionPicker from "$lib/components/RetentionPicker.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import AdvancedReveal from "./AdvancedReveal.svelte";
  import { serializeError } from "./onboarding-mapping";

  let { controller }: { controller: OnboardingController } = $props();

  // Folder picker — mirrors Storage.svelte's Browse logic, writing the chosen
  // path straight into the draft. CLAUDE.md mandates @tauri-apps/plugin-dialog,
  // never the browser-native pickers.
  let browsing = $state(false);

  async function browseSaveDirectory() {
    if (browsing) return;
    browsing = true;
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Choose where Mnema stores captures",
        defaultPath: controller.draftSaveDirectory || undefined,
      });
      if (typeof picked === "string" && picked.trim().length > 0) {
        controller.draftSaveDirectory = picked;
      }
    } catch (err) {
      // The folder picker can reject (dialog plugin error / cancelled host).
      // Surface it via the controller's error banner instead of swallowing it,
      // so the failed Browse… gives visible feedback rather than reading as dead.
      controller.errorMessage = `Couldn't open the folder picker: ${serializeError(err)}`;
    } finally {
      browsing = false;
    }
  }
</script>

<div class="group">
  <div class="group-title">Location</div>

  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Save directory</div>
      <div class="desc">Where captures, the database, and model caches live on disk.</div>
    </div>
    <div class="ctl-field" style="width: 100%">
      <input
        class="input path"
        type="text"
        bind:value={controller.draftSaveDirectory}
        placeholder="Default location (~/.mnema)"
        aria-label="Save directory"
      />
      <button type="button" class="btn" disabled={browsing} onclick={browseSaveDirectory}>
        {browsing ? "Choosing…" : "Browse…"}
      </button>
    </div>
  </div>

  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Retention</div>
      <div class="desc">Captures older than this are cleaned up automatically.</div>
    </div>
    <div class="ctl-field">
      <RetentionPicker bind:value={controller.draftRetentionPolicy} />
    </div>
  </div>
</div>

<div class="group">
  <AdvancedReveal>
    <div class="ctl stack-field">
      <div class="ctl-label">
        <div class="name">Preview cache TTL</div>
        <div class="desc">How long thumbnail previews are kept in memory.</div>
      </div>
      <div class="ctl-field">
        <SelectMenu
          value={String(controller.draftPreviewCacheTtlSeconds)}
          onValueChange={(v) => {
            controller.draftPreviewCacheTtlSeconds = parseInt(v, 10);
          }}
          options={[
            { value: "0", label: "Disabled" },
            { value: "300", label: "5 minutes" },
            { value: "900", label: "15 minutes" },
            { value: "3600", label: "1 hour (default)" },
            { value: "21600", label: "6 hours" },
            { value: "86400", label: "24 hours" },
          ]}
        />
      </div>
    </div>

    <div class="ctl">
      <div class="ctl-label">
        <div class="name">Auto-start on login</div>
        <div class="desc">Begin recording when the app opens.</div>
      </div>
      <div class="ctl-field">
        <Switch bind:checked={controller.draftAutoStart} />
      </div>
    </div>
  </AdvancedReveal>
</div>
