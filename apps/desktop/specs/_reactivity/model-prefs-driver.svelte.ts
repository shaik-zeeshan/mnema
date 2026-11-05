// Drives the REAL ModelPrefs the way a surface does: a reactive settings
// snapshot as its source, constructed inside an effect root.
import { ModelPrefs } from "./modelPrefs.js";

export function makeModelPrefsDriver(initial: unknown) {
  let snapshot = $state.raw<any>(initial);
  let prefs!: ModelPrefs;
  const stop = $effect.root(() => {
    prefs = new ModelPrefs(() => snapshot);
  });
  return {
    prefs,
    setSnapshot(next: unknown) {
      snapshot = next;
    },
    stop,
  };
}
