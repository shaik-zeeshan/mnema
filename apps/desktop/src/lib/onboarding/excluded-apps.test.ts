// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  AUDIO_CLAIM,
  excludedNote,
  excludedSentence,
  resolveTypedApp,
  separatorBefore,
} from "./excluded-apps";

function entry(overrides = {}) {
  return {
    id: "rule-1",
    enabled: true,
    bundleId: "com.apple.Passwords",
    displayName: "Passwords",
    ...overrides,
  };
}

const seeded = [
  entry(),
  entry({ id: "rule-2", bundleId: "com.1password.1password", displayName: "1Password" }),
  entry({ id: "rule-3", bundleId: "com.apple.keychainaccess", displayName: "Keychain Access" }),
];
const pending = entry({ id: "rule-p", bundleId: "", displayName: "Bitwarden" });

describe("separatorBefore", () => {
  it("reads like English at one, two and three names", () => {
    expect([0].map((i) => separatorBefore(i, 1))).toEqual([""]);
    expect([0, 1].map((i) => separatorBefore(i, 2))).toEqual([""," or "]);
    expect([0, 1, 2].map((i) => separatorBefore(i, 3))).toEqual(["", ", ", ", or "]);
    expect([0, 1, 2, 3].map((i) => separatorBefore(i, 4))).toEqual(["", ", ", ", ", ", or "]);
  });
});

describe("excludedSentence", () => {
  it("names the empty state without pretending anything is hidden", () => {
    expect(excludedSentence([])).toEqual({
      lead: "Mnema records everything on your screen.",
      tail: "",
    });
  });

  it("leads with the exclusion while anything is still excluded", () => {
    expect(excludedSentence(seeded).lead).toBe("Mnema never records ");
    expect(excludedSentence([seeded[0], { ...seeded[1], enabled: false }]).lead)
      .toBe("Mnema never records ");
  });

  // The property the whole control rests on: a struck name keeps its slot, so
  // the sentence has to stay grammatical with EVERY name struck rather than
  // dropping the names and reflowing.
  it("keeps every struck name in the sentence, singular and plural", () => {
    const allStruck = seeded.map((app) => ({ ...app, enabled: false }));
    expect(excludedSentence(allStruck)).toEqual({
      lead: "Mnema records everything on your screen. ",
      tail: " are back on.",
    });
    expect(excludedSentence([allStruck[0]]).tail).toBe(" is back on.");
  });
});

describe("excludedNote", () => {
  it("qualifies the audio claim — never unconditional parity", () => {
    expect(excludedNote(seeded).text).toBe(AUDIO_CLAIM);
    expect(AUDIO_CLAIM).toContain("while that filter is on");
  });

  it("says the rule waits, and how many, when an app is not installed yet", () => {
    expect(excludedNote([...seeded, pending]).text).toBe(
      `${AUDIO_CLAIM} One of them isn't installed yet — the rule waits.`,
    );
    expect(excludedNote([...seeded, pending, { ...pending, id: "rule-p2", displayName: "Revolut" }]).text)
      .toBe(`${AUDIO_CLAIM} 2 of them aren't installed yet — the rule waits.`);
  });

  // A pending rule protects nothing (`evaluate_privacy` skips an empty bundle
  // id), so a set that is nothing but pending rules must not claim it does.
  it("never claims protection when every live rule is still pending", () => {
    expect(excludedNote([pending]).text).toBe("One of them isn't installed yet — the rule waits.");
  });

  it("ignores a struck pending rule in the waiting count", () => {
    expect(excludedNote([...seeded, { ...pending, enabled: false }]).text).toBe(AUDIO_CLAIM);
  });

  it("offers the way back in each state", () => {
    expect(excludedNote([]).hint).toBe("Add an app it should never see.");
    expect(excludedNote(seeded).hint).toBe("Click a name to record it after all.");
    expect(excludedNote(seeded.map((a) => ({ ...a, enabled: false }))).hint)
      .toBe("Click a crossed-out name to hide it again.");
  });
});

describe("resolveTypedApp", () => {
  const candidates = [
    { id: "c1", enabled: true, bundleId: "com.apple.Safari", displayName: "Safari", running: true, iconPath: null },
    { id: "c2", enabled: true, bundleId: "com.apple.mail", displayName: "Mail", running: false, iconPath: null },
    { id: "c3", enabled: true, bundleId: "com.apple.MailDrop", displayName: "Mail Drop", running: false, iconPath: null },
  ];

  it("prefers an exact name over a longer prefix match", () => {
    expect(resolveTypedApp("mail", candidates)?.bundleId).toBe("com.apple.mail");
    expect(resolveTypedApp("  Safari ", candidates)?.bundleId).toBe("com.apple.Safari");
  });

  it("falls back to a prefix, and returns null for a name nothing installed matches", () => {
    expect(resolveTypedApp("mail d", candidates)?.bundleId).toBe("com.apple.MailDrop");
    // Null is the "store it pending" signal, not an error.
    expect(resolveTypedApp("Bitwarden", candidates)).toBeNull();
    expect(resolveTypedApp("   ", candidates)).toBeNull();
  });
});
