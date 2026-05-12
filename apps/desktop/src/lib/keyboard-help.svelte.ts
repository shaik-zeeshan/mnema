import type { ShortcutDefinition } from "$lib/keyboard";
import { untrack } from "svelte";

export type KeyboardHelpRow = ShortcutDefinition & {
  enabled?: boolean;
};

export type KeyboardHelpGroup = {
  id: string;
  title: string;
  rows: KeyboardHelpRow[];
};

let groupsByOwner = $state<Record<string, KeyboardHelpGroup[]>>({});

export const keyboardHelp = {
  get contextualGroups(): KeyboardHelpGroup[] {
    return Object.values(groupsByOwner).flat();
  },
};

export function setKeyboardHelpGroups(
  ownerId: string,
  groups: KeyboardHelpGroup[],
): () => void {
  untrack(() => {
    groupsByOwner = {
      ...groupsByOwner,
      [ownerId]: groups,
    };
  });

  return () => {
    untrack(() => {
      const { [ownerId]: _removed, ...remaining } = groupsByOwner;
      groupsByOwner = remaining;
    });
  };
}
