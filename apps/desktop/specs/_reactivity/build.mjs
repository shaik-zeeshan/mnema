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

console.log("compiled");
