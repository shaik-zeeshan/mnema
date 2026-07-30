// Precompile the rune adapter + driver to plain JS with Svelte's compiler.
// Run under NODE (bun can't resolve esrap for svelte/compiler in this repo).
import ts from "typescript";
import { compileModule } from "svelte/compiler";
import { readFileSync, writeFileSync, mkdirSync } from "fs";
import { fileURLToPath } from "url";
import { dirname, resolve } from "path";

const here = dirname(fileURLToPath(import.meta.url));
mkdirSync(resolve(here, "gen"), { recursive: true });
const appRoot = resolve(here, "../..");
const coreAbs = resolve(appRoot, "src/lib/timeline/jumper-cache-core");

function compile(srcPath, outPath, rewrites) {
  let raw = readFileSync(srcPath, "utf8");
  const js = ts.transpileModule(raw, {
    compilerOptions: {
      module: ts.ModuleKind.ESNext,
      target: ts.ScriptTarget.ESNext,
    },
  }).outputText;
  let out = compileModule(js, { filename: srcPath, generate: "client" }).js.code;
  for (const [from, to] of rewrites) out = out.split(from).join(to);
  writeFileSync(outPath, out);
}

// Adapter: point its `./jumper-cache-core` import at the real TS core (absolute,
// bun loads TS on the fly).
compile(
  resolve(appRoot, "src/lib/timeline/jumper-cache.svelte.ts"),
  resolve(here, "gen/jumper-cache.js"),
  [['"./jumper-cache-core"', `"${coreAbs}"`]],
);
// Driver: its `./jumper-cache` import resolves to gen/jumper-cache.js.
compile(resolve(here, "driver.svelte.ts"), resolve(here, "gen/driver.js"), []);

// License store: snapshot-vs-event race regression (licensing-store-race.test.ts).
// Its "$lib/licensing" import is type-only and erased by the TS transpile.
compile(
  resolve(appRoot, "src/lib/licensing-store.svelte.ts"),
  resolve(here, "gen/licensing-store.js"),
  [],
);

// Waveform peaks: per-segment fetch effect + the identity-vs-value dependency
// regression (waveform-peaks.test.ts). Its "$lib/types/app-infra" import is
// type-only and erased by the TS transpile.
compile(
  resolve(appRoot, "src/lib/timeline/waveform-peaks.svelte.ts"),
  resolve(here, "gen/waveform-peaks.js"),
  [],
);

// Onboarding AI store: the verify-probe ordering regression
// (onboarding-ai-verify-race.test.ts). Its two rune dependencies are compiled
// alongside it and re-pointed at the generated files; the plain-TS ones are
// re-pointed at their absolute source (bun loads TS on the fly) and the
// `$lib/types` imports are type-only and erased by the TS transpile.
compile(
  resolve(appRoot, "src/lib/insights/modelPool.svelte.ts"),
  resolve(here, "gen/modelPool.js"),
  [],
);
compile(
  resolve(appRoot, "src/lib/settings/state/ai-runtime.svelte.ts"),
  resolve(here, "gen/ai-runtime.js"),
  [['"$lib/format-error"', `"${resolve(appRoot, "src/lib/format-error")}"`]],
);
compile(
  resolve(appRoot, "src/routes/onboarding/onboarding-ai.svelte.ts"),
  resolve(here, "gen/onboarding-ai.js"),
  [
    ['"$lib/format-error"', `"${resolve(appRoot, "src/lib/format-error")}"`],
    ['"$lib/insights/modelPool.svelte"', '"./modelPool.js"'],
    ['"$lib/onboarding/ai-readiness"', `"${resolve(appRoot, "src/lib/onboarding/ai-readiness")}"`],
    ['"$lib/settings/state/ai-runtime.svelte"', '"./ai-runtime.js"'],
    [
      '"$lib/settings/state/ai-providers"',
      `"${resolve(appRoot, "src/lib/settings/state/ai-providers")}"`,
    ],
  ],
);

console.log("compiled");
