#!/usr/bin/env node
// pi-ask-ai-shim.mjs
//
// Mnema "Ask AI" PI tool shim — seed-only single-answer slice (GitHub issue #70,
// ADR 0024). This runs as a child process of the Mnema desktop app and drives the
// user's installed PI runtime via its SDK. PI's SDK has shipped under more than one
// npm scope (`@mariozechner/pi-coding-agent`, `@earendil-works/pi-coding-agent`), so
// the shim probes the known scopes and an optional override rather than one fixed name.
//
// Contract (must stay in sync with the Rust integrator):
//   - STDIN:  one JSON object read to EOF: { "prompt": "<string>" }.
//             The prompt already contains any seeded context; the shim fetches nothing.
//   - ENV:    MNEMA_PI_EXECUTABLE (optional) — absolute path to the user's `pi` binary,
//             used only to resolve the PI SDK package when it is not on NODE_PATH.
//             MNEMA_PI_SDK_PACKAGE (optional) — overrides/extends the SDK package name
//             tried first, for PI builds published under a different npm scope.
//             PI_CODING_AGENT_DIR (optional) — passed through by the parent; honored
//             transparently by AuthStorage.create(), so this shim does not read it.
//   - STDOUT: newline-delimited JSON, exactly one object per line. Protocol only:
//               {"type":"ready"}                        — session created (optional signal)
//               {"type":"delta","text":"<chunk>"}       — one answer text_delta chunk
//               {"type":"done"}                         — answer complete; then exit 0
//               {"type":"error","message":"<human>"}    — any failure; then exit non-zero
//             Nothing else is ever written to stdout. All diagnostics go to stderr.
//
// This slice exposes NO tools: builtin coding-agent bash/file tools are disabled via
// `noTools: "builtin"`, and no custom Mnema broker tools are registered yet.

import { readFileSync, realpathSync } from "node:fs";
import { dirname, isAbsolute, join } from "node:path";
import { pathToFileURL } from "node:url";
import { createRequire } from "node:module";

// PI's SDK has shipped under multiple npm scopes; probe each known name (plus an
// optional MNEMA_PI_SDK_PACKAGE override, tried first) until one resolves.
const SDK_PACKAGES = (() => {
  const override = process.env.MNEMA_PI_SDK_PACKAGE;
  const known = [
    "@mariozechner/pi-coding-agent",
    "@earendil-works/pi-coding-agent",
  ];
  const ordered = override ? [override, ...known] : known;
  // De-dupe in case the override matches a known scope.
  return ordered.filter((name, index) => ordered.indexOf(name) === index);
})();

// ---- stdout protocol helpers ------------------------------------------------

// Single guard so we never emit a terminal `done`/`error` line twice.
let terminated = false;

/** Write one protocol object as a single newline-terminated JSON line to stdout. */
function emit(obj) {
  // process.stdout.write is synchronous for pipes here; JSON.stringify handles escaping.
  process.stdout.write(JSON.stringify(obj) + "\n");
}

/** Diagnostics never go to stdout — they must not corrupt the protocol stream. */
function diag(...args) {
  console.error("[pi-ask-ai-shim]", ...args);
}

function emitReady() {
  emit({ type: "ready" });
}

function emitDelta(text) {
  if (terminated) return;
  emit({ type: "delta", text });
}

/** Emit the terminal success line at most once and exit 0. */
function finishOk() {
  if (terminated) return;
  terminated = true;
  emit({ type: "done" });
  process.exit(0);
}

/** Emit the terminal error line at most once and exit non-zero. */
function finishError(message) {
  if (terminated) return;
  terminated = true;
  emit({ type: "error", message: String(message) });
  process.exit(1);
}

// ---- stdin -----------------------------------------------------------------

/** Read all of stdin to EOF and return it as a string. */
function readStdin() {
  return new Promise((resolve, reject) => {
    const chunks = [];
    process.stdin.on("data", (c) => chunks.push(c));
    process.stdin.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    process.stdin.on("error", reject);
  });
}

// ---- PI SDK resolution -----------------------------------------------------

/**
 * Resolve a package directory's ESM entry file from its package.json.
 *
 * `import()` of a bare directory does not honor package.json `exports`/`main`, and
 * `require.resolve` fails for ESM-only packages that omit a `require` condition (as
 * PI's SDK does). So when we locate a candidate package directory by walking up from
 * the pi binary, resolve its real entry file ourselves: prefer `exports["."]`
 * (import/default condition), then `module`, then `main`, then `index.js`.
 *
 * Returns an absolute path, or null if package.json is missing/unreadable.
 */
function resolvePackageEntry(pkgDir) {
  let pkg;
  try {
    pkg = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8"));
  } catch (err) {
    diag("could not read package.json at", pkgDir, "-", err?.message ?? err);
    return null;
  }

  const fromExports = (exp) => {
    if (!exp) return null;
    // `exports` may be a bare string, or a subpath map keyed by ".".
    const dot = typeof exp === "string" ? exp : exp["."] ?? null;
    if (typeof dot === "string") return dot;
    if (dot && typeof dot === "object") {
      // Conditional exports: prefer ESM, then a generic default.
      return dot.import ?? dot.module ?? dot.default ?? null;
    }
    return null;
  };

  const relEntry = fromExports(pkg.exports) ?? pkg.module ?? pkg.main ?? "index.js";
  return isAbsolute(relEntry) ? relEntry : join(pkgDir, relEntry);
}

/**
 * Resolve and import the PI SDK package.
 *
 * PI has shipped under more than one npm scope, so every strategy is tried against
 * each candidate name in `SDK_PACKAGES`:
 *   1. Plain dynamic import — works when the SDK is resolvable from NODE_PATH or
 *      global modules.
 *   2. If that fails and MNEMA_PI_EXECUTABLE is set, derive candidate locations
 *      from the real (symlink-resolved) pi binary path and import the first that
 *      resolves.
 *
 * Throws with a clear message if nothing resolves.
 */
async function importPiSdk() {
  // Attempt 1: plain import of each candidate (NODE_PATH / global modules).
  for (const pkg of SDK_PACKAGES) {
    try {
      return await import(pkg);
    } catch (err) {
      diag(`plain import of ${pkg} failed:`, err?.message ?? err);
    }
  }

  // Attempt 2: resolve relative to the real pi binary.
  const piExe = process.env.MNEMA_PI_EXECUTABLE;
  if (piExe) {
    let realPiPath;
    try {
      realPiPath = realpathSync(piExe);
    } catch (err) {
      diag("could not realpath MNEMA_PI_EXECUTABLE:", err?.message ?? err);
      realPiPath = piExe;
    }

    // A require() rooted at the real pi binary path; reused for every candidate.
    let requireFromPi;
    try {
      requireFromPi = createRequire(realPiPath);
    } catch (err) {
      diag("createRequire(realPiPath) failed:", err?.message ?? err);
    }

    // The chain of directories to walk up from the pi binary, computed once.
    const walkupDirs = [];
    {
      let dir = dirname(realPiPath);
      let prev = null;
      while (dir && dir !== prev) {
        walkupDirs.push(dir);
        prev = dir;
        dir = dirname(dir);
      }
    }

    for (const pkg of SDK_PACKAGES) {
      const candidateUrls = [];

      // 2a: require.resolve from the perspective of the real pi binary path.
      if (requireFromPi) {
        try {
          candidateUrls.push(requireFromPi.resolve(pkg));
        } catch (err) {
          diag(`createRequire(realPiPath).resolve(${pkg}) failed:`, err?.message ?? err);
        }
      }

      // 2b: walk up from the pi binary dir looking for sibling node_modules,
      // resolving each candidate package directory to its real ESM entry file
      // (a bare directory import does not honor package.json exports/main).
      for (const baseDir of walkupDirs) {
        const pkgDir = join(baseDir, "node_modules", ...pkg.split("/"));
        const entry = resolvePackageEntry(pkgDir);
        if (entry) {
          candidateUrls.push(entry);
        }
      }

      for (const candidate of candidateUrls) {
        try {
          // Import via file URL so absolute paths resolve on every platform.
          const specifier = isAbsolute(candidate)
            ? pathToFileURL(candidate).href
            : candidate;
          return await import(specifier);
        } catch (err) {
          diag("candidate import failed:", candidate, "-", err?.message ?? err);
        }
      }
    }
  }

  throw new Error(
    `Could not resolve PI's SDK package (tried ${SDK_PACKAGES.join(", ")}). Ensure the ` +
      "PI runtime is installed and either on NODE_PATH or co-located with the pi binary " +
      "(set MNEMA_PI_EXECUTABLE to the pi binary path).",
  );
}

// ---- main ------------------------------------------------------------------

async function main() {
  // 1. Read + parse stdin config.
  const raw = await readStdin();
  let prompt;
  try {
    const config = JSON.parse(raw);
    prompt = config?.prompt;
  } catch (err) {
    throw new Error(`Failed to parse stdin JSON config: ${err?.message ?? err}`);
  }
  if (typeof prompt !== "string" || prompt.length === 0) {
    throw new Error('stdin config must be a JSON object with a non-empty string "prompt".');
  }

  // 2. Resolve + import the PI SDK.
  const sdk = await importPiSdk();
  const { AuthStorage, ModelRegistry, createAgentSession, SessionManager } = sdk;
  if (!AuthStorage || !ModelRegistry || !createAgentSession || !SessionManager) {
    throw new Error(
      "PI SDK was imported but is missing expected exports " +
        "(AuthStorage, ModelRegistry, createAgentSession, SessionManager).",
    );
  }

  // 3. Build auth + model registry from the user's existing PI config.
  const authStorage = AuthStorage.create();
  const modelRegistry = ModelRegistry.create(authStorage);

  // Resolve the user's configured default model, if the registry exposes one.
  // Be defensive: the helper name may vary across PI releases, so probe a few.
  let defaultModel;
  try {
    if (typeof modelRegistry.getDefaultModel === "function") {
      defaultModel = modelRegistry.getDefaultModel();
    } else if (typeof modelRegistry.defaultModel === "function") {
      defaultModel = modelRegistry.defaultModel();
    } else if (modelRegistry.defaultModel) {
      defaultModel = modelRegistry.defaultModel;
    }
  } catch (err) {
    diag("default model lookup failed:", err?.message ?? err);
  }

  // 4. Create the session. No builtin tools, no custom tools, ephemeral in-memory.
  const sessionOptions = {
    noTools: "builtin",
    authStorage,
    modelRegistry,
    sessionManager: SessionManager.inMemory(),
  };
  if (defaultModel) {
    sessionOptions.model = defaultModel;
  }

  let session;
  try {
    const created = await createAgentSession(sessionOptions);
    session = created?.session ?? created;
  } catch (err) {
    const msg = String(err?.message ?? err);
    // A common failure here is no configured provider/model.
    if (/model|provider|auth|credential/i.test(msg) && !defaultModel) {
      throw new Error(
        "No PI provider/model is configured. Configure a default model in your " +
          `PI runtime before using Ask AI. (underlying: ${msg})`,
      );
    }
    throw new Error(`Failed to create PI agent session: ${msg}`);
  }
  if (!session || typeof session.subscribe !== "function" || typeof session.prompt !== "function") {
    throw new Error("PI agent session is missing expected subscribe/prompt methods.");
  }

  emitReady();

  // 5. Subscribe BEFORE prompting so we never miss early streamed tokens.
  session.subscribe((event) => {
    if (terminated) return;
    try {
      const type = event?.type;

      // The whole run finished — terminal.
      if (type === "agent_end") {
        finishOk();
        return;
      }

      // Streaming message updates carry the assistant message sub-events.
      if (type === "message_update") {
        const ame = event.assistantMessageEvent;
        if (!ame) return;
        switch (ame.type) {
          case "text_delta":
            // Forward only answer text; `delta` is the raw chunk string.
            if (typeof ame.delta === "string") emitDelta(ame.delta);
            return;
          case "thinking_delta":
            // Thinking tokens are NOT answer text — never forward them.
            return;
          case "done":
            // One assistant message finished. Treat as completion for this
            // single-answer slice (agent_end may or may not follow).
            finishOk();
            return;
          case "error": {
            const reason = ame.reason ?? "error";
            const detail = ame.error ? ` (${String(ame.error)})` : "";
            finishError(`PI assistant message ${reason}${detail}`);
            return;
          }
          default:
            return;
        }
      }
    } catch (err) {
      finishError(`Error handling PI agent event: ${err?.message ?? err}`);
    }
  });

  // 6. Drive the prompt. The terminal line is emitted from the subscription,
  //    but guard here too in case the run resolves without a terminal event.
  try {
    await session.prompt(prompt);
  } catch (err) {
    finishError(`PI prompt failed: ${err?.message ?? err}`);
    return;
  }

  // If the SDK resolved prompt() without ever sending a terminal event, finish ok.
  finishOk();
}

// Top-level safety net: any uncaught error becomes a protocol error line.
process.on("uncaughtException", (err) => {
  finishError(`Uncaught exception: ${err?.message ?? err}`);
});
process.on("unhandledRejection", (err) => {
  finishError(`Unhandled rejection: ${err?.message ?? err}`);
});

main().catch((err) => {
  finishError(err?.message ?? String(err));
});
