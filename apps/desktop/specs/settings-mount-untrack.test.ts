import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The mount $effect in settings/+page.svelte MUST call its loaders inside
// `untrack(() => { ... })`. Several loaders synchronously read draft `$state`
// (e.g. refreshAiProviderKeyPresence reads the draft provider list); without
// untrack the effect would subscribe to those drafts and re-fire on every edit,
// clobbering the in-flight draft back to the persisted value before autosave.
// (See the "Mnema settings init effect untrack" project invariant.)
//
// A full component test needs a Svelte runtime harness that isn't wired into
// this bun:test setup, so we assert the STRUCTURAL guarantee against the source:
// every mount loader call sits inside the untrack() block. This is the most
// faithful check the available harness allows.

const pagePath = fileURLToPath(
  new URL("../src/routes/settings/+page.svelte", import.meta.url),
);
const source = readFileSync(pagePath, "utf8");

// Extract the body of the mount-init `untrack(() => { ... })`. There can be
// several `untrack(...)` uses in the file (e.g. a small model-pool reset), so we
// pick the one whose body contains the mount marker `loadCaptureSupport(`.
function mountUntrackBlock(src: string): string {
  let searchFrom = 0;
  for (;;) {
    const start = src.indexOf("untrack(() => {", searchFrom);
    expect(start).toBeGreaterThan(-1);
    const open = src.indexOf("{", start);
    let depth = 0;
    let body = "";
    for (let i = open; i < src.length; i++) {
      if (src[i] === "{") depth++;
      else if (src[i] === "}") {
        depth--;
        if (depth === 0) {
          body = src.slice(open + 1, i);
          break;
        }
      }
    }
    if (body.includes("loadCaptureSupport(")) return body;
    searchFrom = start + "untrack(() => {".length;
  }
}

describe("settings mount: loaders run inside untrack()", () => {
  const block = mountUntrackBlock(source);

  // The mount loaders that must be inside untrack — at least the draft-reading
  // ones that motivated the invariant, plus the broad initial load set.
  const mountLoaders = [
    "loadRecordingSettings",
    "refreshAiProviderKeyPresence",
    "loadAiRuntimeStatus",
    "refreshUserContext",
    "loadCaptureSupport",
    "loadKeyboardBindingsSettings",
    "loadMicState",
    "loadOcrModelStatus",
    "loadTranscriptionModelStatus",
    "loadSpeakerModelStatus",
    "loadSemanticSearchModelStatus",
    "loadSemanticSearchSupportedModels",
    "loadPersonProfileCount",
    "loadDebugLogStatus",
    "loadGeneralLogStatus",
    "loadAppUpdateStatus",
    "loadThirdPartyNotices",
    "loadBrokerGrants",
    "loadMnemaCliStatus",
    "loadAskAiAvailability",
  ];

  for (const loader of mountLoaders) {
    test(`${loader}() is invoked inside the mount untrack block`, () => {
      expect(block).toContain(`${loader}(`);
    });
  }

  test("the draft-reading provider-key refresh is chained off loadRecordingSettings inside untrack", () => {
    // Specifically the invariant's motivating case: provider-key presence is
    // refreshed only after recording settings load, and the whole thing is in
    // the untrack block.
    //
    // loadRecordingSettings now lives on the recording-settings store (Slice 3
    // core cutover), so it is invoked as `rec.loadRecordingSettings()`. The
    // guarantee is unchanged — the recording-load → key-refresh chain is still
    // inside untrack; only the call site is now store-qualified. Asserting the
    // `rec.`-prefixed shape keeps the check from being satisfied by an unrelated
    // bare `loadRecordingSettings` token elsewhere in the block.
    expect(block).toContain("rec.loadRecordingSettings().then(() => refreshAiProviderKeyPresence())");
  });
});
