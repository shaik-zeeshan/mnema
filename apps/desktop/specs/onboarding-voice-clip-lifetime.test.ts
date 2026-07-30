import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// `enroll_account_owner_voice` DESTROYS the clip it judges, on every exit —
// stored, rejected, or failed (src-tauri/src/voice_enrollment.rs,
// `embed_enrollment_clip_for_build`: `let _ = std::fs::remove_file(clip_path)`).
// The backend's playback affordance is therefore only valid BEFORE that call
// ("the enrollment surface plays the take back before committing to it" — the
// asset-scope grant in `record_bounded_microphone_clip`). The Settings door
// honours it: `VoiceEnrollmentStore` has a `review` stage and nulls `clipPath`
// the moment `enroll_account_owner_voice` returns.
//
// The onboarding Voice screen enrolls IMMEDIATELY after recording, so any clip
// path it keeps afterwards names a file the backend has already deleted, and a
// playback control gated on that path can never work.
//
// A full component test needs a Svelte runtime harness that isn't wired into
// this bun:test setup, so we assert the STRUCTURAL guarantee against the source
// (same approach as settings-mount-untrack.test.ts).

const screenPath = fileURLToPath(
  new URL("../src/routes/onboarding/screens/VoiceScreen.svelte", import.meta.url),
);
const source = readFileSync(screenPath, "utf8");

/** Body of the named function in the source, brace-matched. */
function functionBody(src: string, name: string): string {
  const start = src.indexOf(`function ${name}(`);
  expect(start).toBeGreaterThan(-1);
  const open = src.indexOf("{", start);
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === "{") depth++;
    else if (src[i] === "}") {
      depth--;
      if (depth === 0) return src.slice(open + 1, i);
    }
  }
  throw new Error(`unbalanced braces in ${name}`);
}

test("the onboarding take loop hands the clip over and does not keep the path", () => {
  const body = functionBody(source, "recordTake");
  expect(body).toContain("enroll_account_owner_voice");
  expect(
    /clipPath\s*=\s*path/.test(body),
    "recordTake stores the clip path in component state, but enroll_account_owner_voice deletes the file",
  ).toBe(false);
});

test("the onboarding voice screen offers no playback of a destroyed clip", () => {
  expect(
    /convertFileSrc/.test(source),
    "the screen builds an asset URL for an enrollment clip the backend has already deleted",
  ).toBe(false);
});
