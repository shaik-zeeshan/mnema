<script lang="ts">
  // Settings toolbar tabs — direction 05 "Tactile Instruments".
  //
  // This direction's answer to the per-direction settings-navigation decision:
  // THE LEFT RAIL IS GONE. Top-level sections are a native toolbar of
  // icon+label tabs; inside a tab there is ONE 660px scrolling column with
  // opaque sticky group headers. Navigation is horizontal at the top and
  // vertical in the content, never both at once.
  //
  // The strip also carries the two things that must never be clippable: the
  // scoped ⌘F field, and the autosave chip (G7 — no bottom save bar, ever).
  //
  // Behaviour is unchanged from the rail it replaces: a tab activates the
  // group's first section through the same `onNavigate` the shell already
  // owned, so `groups.ts` deeplinks, scroll-spy and the mounted-panel switch
  // all work exactly as before. Only the shape moved.

  import { SETTINGS_GROUPS, type SettingsGroupId } from "./groups";
  import { SECTION_ICONS } from "./section-icons";
  import SettingsFindBar from "./ui/SettingsFindBar.svelte";
  import SettingsSaveChip from "./ui/SettingsSaveChip.svelte";

  interface Props {
    /** The active group — the one group panel currently mounted. */
    activeGroup: SettingsGroupId;
    /** Called on tab activation; the shell scrolls to that section. */
    onNavigate: (section: (typeof SETTINGS_GROUPS)[number]["sections"][number]["id"]) => void;
  }

  let { activeGroup, onNavigate }: Props = $props();

  // A group's glyph is its first section's glyph — the sections already own the
  // shared Lucide family, so the tab bar borrows rather than inventing a second
  // icon set to keep in sync.
  function groupIcon(group: (typeof SETTINGS_GROUPS)[number]) {
    return SECTION_ICONS[group.sections[0].id];
  }

  // Arrow keys step the tab strip; this is a real tablist-shaped toolbar, so
  // left/right is the expected axis (the rail's up/down went with it).
  function onKeydown(event: KeyboardEvent) {
    const delta = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (delta === 0) return;
    event.preventDefault();
    const i = SETTINGS_GROUPS.findIndex((g) => g.id === activeGroup);
    if (i === -1) return;
    const next = SETTINGS_GROUPS[(i + delta + SETTINGS_GROUPS.length) % SETTINGS_GROUPS.length];
    onNavigate(next.sections[0].id);
    document.getElementById(`settings-tab-${next.id}`)?.focus();
  }
</script>

<div class="ti-tabbar settings-tabbar" role="toolbar" aria-label="Settings sections">
  {#each SETTINGS_GROUPS as group (group.id)}
    {@const GroupIcon = groupIcon(group)}
    <button
      class="ti-tabb"
      class:is-on={activeGroup === group.id}
      type="button"
      id="settings-tab-{group.id}"
      aria-current={activeGroup === group.id ? "page" : undefined}
      tabindex={activeGroup === group.id ? 0 : -1}
      onclick={() => onNavigate(group.sections[0].id)}
      onkeydown={onKeydown}
    >
      <GroupIcon aria-hidden="true" />
      <span>{group.label}</span>
    </button>
  {/each}

  <SettingsFindBar />
  <SettingsSaveChip />
</div>

<style>
  /* The strip is pinned to the top of the window at every size, so neither the
     save chip nor the filter field can fall off a short window. */
  .settings-tabbar {
    position: relative;
    z-index: 6;
  }

  /* The shared Lucide glyphs carry no intrinsic size; the tab owns it. */
  .settings-tabbar :global(.ti-tabb svg) {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
