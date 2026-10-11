<script lang="ts">
  // The 52px content header of the full-window Settings mode (settings.html
  // `.st-bar`): a drag region with the breadcrumb (group › section in view) and
  // the one auto-save status. Priority: save failed > resolve issues > saving >
  // saved. The failure's message + Retry/Dismiss render as a notice at the top
  // of the content (settings/+page.svelte); here it is one line + Retry.
  import IconRetry from "~icons/lucide/rotate-cw";
  import { SETTINGS_GROUPS, type SettingsGroupId, type SettingsSectionId } from "./groups";
  import { getSettingsController } from "./state/controller.svelte";

  interface Props {
    activeGroup: SettingsGroupId;
    activeSection: SettingsSectionId;
    /** The content pane is scrolled: the bar gains a hairline. */
    scrolled: boolean;
  }
  let { activeGroup, activeSection, scrolled }: Props = $props();

  const c = getSettingsController();
  const group = $derived(SETTINGS_GROUPS.find((g) => g.id === activeGroup)!);
  // The crumb adds the section only once you're past the group's first one.
  const section = $derived(
    group.sections[0].id === activeSection ? null : group.sections.find((s) => s.id === activeSection)?.label ?? null,
  );

  const status = $derived(c.saveStatus);
</script>

<header class="st-bar" class:is-scrolled={scrolled} data-tauri-drag-region>
  <div class="st-bar__crumb" data-tauri-drag-region>
    <b>{group.label}</b>{#if section}<span>{section}</span>{/if}
  </div>
  {#if status === "error"}
    <span class="st-save is-failed" role="alert">
      Couldn’t save
      {#if c.rec.recError && c.lastFailedSaveDomain}
        <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={() => c.retryFailedSave()}>
          <IconRetry width="12" height="12" aria-hidden="true" />Retry
        </button>
      {/if}
    </span>
  {:else}
    <span class="st-save" class:is-saving={status !== "saved"} role="status">
      {status === "blocked" ? "Resolve issues to save" : status === "saving" ? "Saving…" : "All changes saved"}
    </span>
  {/if}
</header>

<style>
  .st-bar {
    flex: none;
    height: 52px;
    display: flex;
    align-items: center;
    gap: var(--s-3);
    padding: 0 var(--s-5) 0 var(--s-7);
    border-bottom: 1px solid transparent;
    user-select: none;
    transition: border-color var(--t-med) var(--ease-quart), background-color var(--t-med) var(--ease-quart);
  }
  .st-bar.is-scrolled {
    border-bottom-color: var(--mx-hairline);
    background: color-mix(in srgb, var(--app-bg) 80%, transparent);
  }
  .st-bar__crumb {
    display: flex;
    align-items: center;
    gap: 6px;
    font: 500 var(--text-md)/1 var(--font-sans);
    color: var(--app-text-strong);
  }
  .st-bar__crumb b {
    font-weight: 600;
  }
  .st-bar__crumb span {
    color: var(--app-text-subtle);
    font-weight: 400;
  }
  .st-bar__crumb span::before {
    content: "›";
    margin-right: 6px;
    color: var(--app-text-faint);
  }
  .st-save {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: 400 var(--text-sm)/1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .st-save::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-source-mic);
  }
  .st-save.is-saving {
    color: var(--app-text);
  }
  .st-save.is-saving::before {
    background: var(--app-warn);
  }
  .st-save.is-failed {
    color: var(--app-danger-text);
  }
  .st-save.is-failed::before {
    background: var(--app-danger);
  }
  .st-save .mx-btn {
    --_h: 22px;
  }
</style>
