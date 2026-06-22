import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { Glob } from "bun";

// Slice-6 acceptance guard (project invariant: keep every source file under
// 800 lines). The Settings refactor split a 9989-line monolith into a thin
// shell + section panels + state stores; this test fails loudly if any SOURCE
// file in the settings tree drifts back over the cap so the split is never
// silently re-monolithised.
//
// SCOPE: `.svelte` component source and `.svelte.ts` rune-state modules under
// `lib/settings` and `routes/settings`. Stylesheets (`.css`) are intentionally
// excluded — CSS has its own cohesive-split treatment (settings.css is a thin
// @import manifest over `settings-{layout,groups,controls,blocks,theme}.css`),
// and a hard line cap on a verbatim-namespaced stylesheet is the wrong axis.

const MAX_LINES = 800;

const roots = [
  fileURLToPath(new URL("../src/lib/settings", import.meta.url)),
  fileURLToPath(new URL("../src/routes/settings", import.meta.url)),
];

function sourceFiles(): string[] {
  const out: string[] = [];
  // `.svelte.ts` also matches `*.ts`; the `.svelte`/`.svelte.ts` filter below
  // keeps it to component + rune-state source (plain `.ts` helpers included too,
  // which is fine — the cap applies to all hand-written source).
  const glob = new Glob("**/*.{svelte,ts}");
  for (const root of roots) {
    for (const rel of glob.scanSync({ cwd: root })) {
      out.push(`${root}/${rel}`);
    }
  }
  return out;
}

describe("settings tree: 800-line source-file cap", () => {
  test("no .svelte/.svelte.ts/.ts source file exceeds 800 lines", () => {
    const offenders: string[] = [];
    for (const path of sourceFiles()) {
      const lines = readFileSync(path, "utf8").split("\n").length;
      if (lines > MAX_LINES) {
        offenders.push(`${path} (${lines} lines)`);
      }
    }
    expect(offenders).toEqual([]);
  });
});
