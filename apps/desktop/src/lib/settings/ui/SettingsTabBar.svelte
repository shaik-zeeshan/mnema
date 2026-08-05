<script lang="ts">
  // Direction 01 (Bento Native) — the settings top level is a TOOLBAR OF TABS.
  //
  // The left rail is gone (mockup 05/06): five tabs carry the five groups, and
  // this same sticky sub-bar carries the two things that must never scroll away
  // — the autosave chip (G7: top-anchored, no bottom save bar, ever) and the
  // scoped ⌘F field. Sub-sections are no longer navigable on their own; a group
  // is one short bento pane, so the scroll-spy the rail needed is gone with it.
  //
  // A real tablist: each tab controls the one mounted group panel
  // (`settings-panel-<group>`), so roving tabindex + arrow/Home/End is the
  // correct keyboard model here (unlike the rail, which was a nav landmark).

  import { goto } from "$app/navigation";
  import IconBack from "~icons/lucide/chevron-left";
  import IconGeneral from "~icons/lucide/sliders-horizontal";
  import IconCapture from "~icons/lucide/monitor";
  import IconIntelligence from "~icons/lucide/sparkles";
  import IconData from "~icons/lucide/database";
  import IconAbout from "~icons/lucide/info";
  import { SETTINGS_GROUPS, type SettingsGroupId } from "../groups";
  import { getLastMainSurface } from "$lib/surface-windows";
  import SettingsSaveChip from "./SettingsSaveChip.svelte";
  import SettingsFindBar from "./SettingsFindBar.svelte";

  interface Props {
    activeGroup: SettingsGroupId;
    onSelect: (group: SettingsGroupId) => void;
  }

  let { activeGroup, onSelect }: Props = $props();

  const GROUP_ICONS: Record<SettingsGroupId, typeof IconGeneral> = {
    general: IconGeneral,
    capture: IconCapture,
    intelligence: IconIntelligence,
    data: IconData,
    about: IconAbout,
  };

  // Settings is the `/settings` route inside the Main window, so leaving it is a
  // plain in-window navigation back to whichever main surface was last shown.
  function backToApp() {
    void goto(getLastMainSurface());
  }

  function handleKeydown(event: KeyboardEvent) {
    const ids = SETTINGS_GROUPS.map((g) => g.id);
    const current = ids.indexOf(activeGroup);
    let next: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") next = (current + 1) % ids.length;
    else if (event.key === "ArrowLeft" || event.key === "ArrowUp") next = (current - 1 + ids.length) % ids.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = ids.length - 1;
    if (next === null) return;
    event.preventDefault();
    const id = ids[next]!;
    onSelect(id);
    document.getElementById(`settings-tab-${id}`)?.focus();
  }
</script>

<div class="setbar">
  <button class="setbar__back" type="button" onclick={backToApp} aria-label="Back to app">
    <IconBack aria-hidden="true" />
  </button>

  <div class="setbar__tabs" role="tablist" aria-label="Settings sections">
    {#each SETTINGS_GROUPS as group (group.id)}
      {@const GroupIcon = GROUP_ICONS[group.id]}
      <button
        class="stab"
        class:stab--on={activeGroup === group.id}
        type="button"
        role="tab"
        id="settings-tab-{group.id}"
        aria-selected={activeGroup === group.id}
        aria-controls="settings-panel-{group.id}"
        tabindex={activeGroup === group.id ? 0 : -1}
        onclick={() => onSelect(group.id)}
        onkeydown={handleKeydown}
      >
        <GroupIcon aria-hidden="true" />
        <span>{group.label}</span>
      </button>
    {/each}
  </div>

  <SettingsSaveChip />
  <SettingsFindBar />
</div>
