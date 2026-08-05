<script lang="ts">
  // Settings tab bar — direction 04 "Command Deck".
  //
  // The Codex left rail is gone. Five horizontal tabs sit above one scroll
  // region with sticky section headers, and the ⌘F filter field lives at the
  // right end of the same strip (it IS the navigation — see SettingsFindBar).
  //
  // Section keys are ⌃1–⌃5, NOT ⌘1–⌘5: ⌘1/⌘2 is the global Timeline/Overview
  // surface switcher and must stay global. Nothing else in the app binds
  // Control+digit (the app's plain 1/2/3 source toggles bail on any modifier).
  //
  // Still a NAVIGATION landmark rather than a tablist: with ⌘F active all five
  // panels mount at once, so "one tab controls one panel" is not true and a
  // real tablist would be lying. The active tab carries aria-current="page".
  //
  // This component also owns the deck's context + hints for the whole route
  // (G7 puts the autosave STATUS there too — see ui/SettingsSaveState.svelte).

  import { goto } from "$app/navigation";
  import IconBack from "~icons/lucide/chevron-left";
  import { GROUP_ICONS } from "./section-icons";
  import {
    SETTINGS_GROUPS,
    type SettingsGroupId,
    type SettingsSectionId,
  } from "./groups";
  import { SETTINGS_ROW_INDEX, matchingRowCount } from "./settings-index";
  import { settingsFind } from "./state/settings-find.svelte";
  import { getSettingsController } from "./state/controller.svelte";
  import SettingsFindBar from "./ui/SettingsFindBar.svelte";
  import { setDeck, resetDeck, type DeckHint } from "$lib/deck.svelte";
  import { getLastMainSurface } from "$lib/surface-windows";

  interface Props {
    activeGroup: SettingsGroupId;
    /** Called with the group's entry section when a tab is chosen. */
    onNavigate: (section: SettingsSectionId) => void;
  }

  let { activeGroup, onNavigate }: Props = $props();

  const c = getSettingsController();

  /** The keycap each tab wears, and the key that actually switches to it. */
  const TAB_KEYS = ["⌃1", "⌃2", "⌃3", "⌃4", "⌃5"] as const;

  function selectGroup(group: SettingsGroupId) {
    const entry = SETTINGS_GROUPS.find((g) => g.id === group)?.sections[0]?.id;
    if (entry) onNavigate(entry);
  }

  // ⌃1–⌃5. Skipped while a shortcut rebind is listening (it swallows keys on a
  // window capture-phase listener) and while typing in a field.
  function onKeydown(event: KeyboardEvent) {
    if (!event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
    if (c.keyboard.shortcutCaptureActionId !== null) return;
    const index = TAB_KEYS.findIndex((_, i) => event.key === String(i + 1));
    if (index === -1) return;
    const group = SETTINGS_GROUPS[index];
    if (!group) return;
    event.preventDefault();
    selectGroup(group.id);
  }

  $effect(() => {
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });

  // ── The deck (G7): context + hints. The autosave status slot is published
  // separately by `ui/SettingsSaveState.svelte`; `setDeck` merges partials.
  const query = $derived(settingsFind.query.trim());
  const hits = $derived(settingsFind.active ? matchingRowCount(query) : 0);
  const activeLabel = $derived(
    SETTINGS_GROUPS.find((g) => g.id === activeGroup)?.label ?? "",
  );

  // Only keys that really work are advertised. No ⌘Z: Settings-Undo is out for
  // v1 (G7), so the mockup's "⌘Z Undo" hint is deliberately not shipped.
  const hints = $derived<DeckHint[]>(
    settingsFind.active
      ? [
          { keys: "↑↓", label: "Move" },
          { keys: "␣", label: "Toggle" },
          { keys: "esc", label: "Clear filter", separator: true },
        ]
      : [
          { keys: "⌘F", label: "Filter" },
          { keys: "⌃1–5", label: "Sections" },
          { keys: "↑↓", label: "Move" },
          { keys: "␣", label: "Toggle" },
        ],
  );

  $effect(() => {
    setDeck({
      context: settingsFind.active
        ? `Settings · filtering “${query}” · ${hits} of ${SETTINGS_ROW_INDEX.length}`
        : `Settings · ${activeLabel}`,
      hints,
    });
  });

  // Cleanup only — an effect with no tracked reads runs once and its teardown
  // fires on unmount, so leaving Settings clears the whole deck (status too).
  $effect(() => resetDeck);
</script>

<nav class="stabs" aria-label="Settings sections">
  <button
    class="stabs__back"
    type="button"
    onclick={() => void goto(getLastMainSurface())}
    aria-label="Back to app"
  >
    <IconBack aria-hidden="true" />
  </button>

  {#each SETTINGS_GROUPS as group, i (group.id)}
    {@const GroupIcon = GROUP_ICONS[group.id]}
    <button
      class="stab"
      class:stab--on={activeGroup === group.id}
      type="button"
      id="settings-tab-{group.id}"
      aria-current={activeGroup === group.id ? "page" : undefined}
      onclick={() => selectGroup(group.id)}
    >
      <GroupIcon aria-hidden="true" />
      {group.label}
      <span class="kbd" aria-hidden="true">{TAB_KEYS[i]}</span>
    </button>
  {/each}

  <SettingsFindBar {hits} />
</nav>
