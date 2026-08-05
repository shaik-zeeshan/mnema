import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { Glob } from "bun";
import {
  SETTINGS_ROW_INDEX,
  rowMatchesQuery,
} from "../src/lib/settings/settings-index";

// ── G7's index-completeness tripwire ────────────────────────────────────────
//
// "⌘F row-filtering is IN: every settings row indexed (label + synonyms +
// section). Index completeness is enforced by a test — every registered setting
// must have an index entry." (docs/redesign/round4/DECISIONS.md, G7)
//
// The enumerable source of truth is the panel markup: a settings row IS a
// `<SettingRow label="…">` inside a panel section component, and each section
// component declares its section once with `setSettingsSection("…")` (the same
// call `SettingRow` reads at runtime via context). So this test re-derives the
// full row set from source and fails if the index and the markup disagree — in
// EITHER direction: a new row with no entry can't ship, and an entry left behind
// by a deleted/renamed row can't rot in the index unnoticed.

const panelsRoot = fileURLToPath(
  new URL("../src/lib/settings/panels", import.meta.url),
);

interface SourceRow {
  file: string;
  section: string;
  label: string;
}

/** Labels on one `<SettingRow …>` open tag: `label="X"`, or the string literals
 *  inside a `label={…}` expression (License's `cond ? "Renew" : "Buy Mnema"`
 *  registers both). */
function labelsFromTag(tag: string): string[] {
  const literal = tag.match(/\blabel="([^"]*)"/);
  if (literal) return [literal[1]];
  const expression = tag.match(/\blabel=\{([^{}]*)\}/);
  if (expression) {
    return [...expression[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  }
  return [];
}

function scanPanelRows(): { rows: SourceRow[]; problems: string[] } {
  const rows: SourceRow[] = [];
  const problems: string[] = [];
  const glob = new Glob("**/*.svelte");
  for (const rel of [...glob.scanSync({ cwd: panelsRoot })].sort()) {
    const source = readFileSync(`${panelsRoot}/${rel}`, "utf8");
    if (!source.includes("<SettingRow")) continue;

    const section = source.match(/setSettingsSection\("([^"]+)"\)/)?.[1];
    if (!section) {
      problems.push(
        `${rel} renders <SettingRow> but never calls setSettingsSection("…"). ` +
          `Add it at the top of the <script> so its rows know their section ` +
          `(⌘F breadcrumb + index key).`,
      );
      continue;
    }

    for (const match of source.matchAll(/<SettingRow\b([\s\S]*?)>/g)) {
      const labels = labelsFromTag(match[1]);
      if (labels.length === 0) {
        problems.push(
          `${rel}: a <SettingRow> has no readable label. Give it a literal ` +
            `label="…" (a fully dynamic label cannot be indexed).`,
        );
        continue;
      }
      for (const label of labels) rows.push({ file: rel, section, label });
    }
  }
  return { rows, problems };
}

const { rows, problems } = scanPanelRows();

describe("settings row index completeness (G7)", () => {
  test("every panel file with rows declares its section, with readable labels", () => {
    expect(problems).toEqual([]);
  });

  test("the scan found the panel rows at all (guards a vacuous pass)", () => {
    // If the markup shape ever changes so the scan matches nothing, this test —
    // not a silently-green completeness check — is what fails.
    expect(rows.length).toBeGreaterThan(80);
  });

  test("every rendered settings row has an index entry", () => {
    const indexed = new Set(
      SETTINGS_ROW_INDEX.map((entry) => `${entry.section} ${entry.label}`),
    );
    const missing = rows
      .filter((row) => !indexed.has(`${row.section} ${row.label}`))
      .map(
        (row) =>
          `${row.file}: row "${row.label}" is not in the ⌘F index. Add to ` +
          `src/lib/settings/settings-index.ts:\n` +
          `  { section: "${row.section}", label: "${row.label}", synonyms: [/* words a user would type */] },`,
      );
    expect(missing).toEqual([]);
  });

  test("no index entry outlives the row it indexes", () => {
    const rendered = new Set(rows.map((row) => `${row.section} ${row.label}`));
    const orphans = SETTINGS_ROW_INDEX.filter(
      (entry) => !rendered.has(`${entry.section} ${entry.label}`),
    ).map(
      (entry) =>
        `{ section: "${entry.section}", label: "${entry.label}" } indexes no ` +
        `rendered row — the row was renamed or deleted. Update or remove the ` +
        `entry in src/lib/settings/settings-index.ts.`,
    );
    expect(orphans).toEqual([]);
  });

  test("index keys are unique", () => {
    const seen = new Set<string>();
    const duplicates: string[] = [];
    for (const entry of SETTINGS_ROW_INDEX) {
      const key = `${entry.section} ${entry.label}`;
      if (seen.has(key)) duplicates.push(key);
      seen.add(key);
    }
    expect(duplicates).toEqual([]);
  });
});

describe("⌘F matcher", () => {
  test("matches the row label, case-insensitively", () => {
    expect(rowMatchesQuery("appearance", "Theme", "them")).toBe(true);
    expect(rowMatchesQuery("appearance", "Theme", "THEME")).toBe(true);
  });

  test("matches indexed synonyms the label doesn't carry", () => {
    // "vad" is nowhere in the label or the section name.
    expect(
      rowMatchesQuery("capture", "Microphone Voice Detection", "vad"),
    ).toBe(true);
    expect(rowMatchesQuery("storage", "Retention", "delete old")).toBe(true);
  });

  test("matches the section and group name, so a section query keeps its rows", () => {
    expect(rowMatchesQuery("video", "Bitrate", "video")).toBe(true);
    expect(rowMatchesQuery("video", "Bitrate", "capture")).toBe(true);
  });

  test("every query token must hit, so 'ocr lang' finds OCR's Language row", () => {
    expect(rowMatchesQuery("ocr", "Language", "ocr lang")).toBe(true);
    // …and not Transcription's, which is a different section.
    expect(rowMatchesQuery("transcription", "Language", "ocr lang")).toBe(false);
  });

  test("an unmatched token fails the row", () => {
    expect(rowMatchesQuery("appearance", "Theme", "theme bitrate")).toBe(false);
    expect(rowMatchesQuery("appearance", "Theme", "zzz")).toBe(false);
  });

  test("an empty query matches nothing (callers render the normal panels)", () => {
    expect(rowMatchesQuery("appearance", "Theme", "")).toBe(false);
    expect(rowMatchesQuery("appearance", "Theme", "   ")).toBe(false);
  });

  test("a row outside a settings panel (no section context) still matches its label", () => {
    expect(rowMatchesQuery(null, "Theme", "theme")).toBe(true);
    expect(rowMatchesQuery(null, "Theme", "appearance")).toBe(false);
  });
});
