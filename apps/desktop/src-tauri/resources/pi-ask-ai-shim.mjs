#!/usr/bin/env node
// pi-ask-ai-shim.mjs
//
// Mnema "Ask AI" PI tool shim — tool-enabled answer slice (GitHub issue #70,
// ADR 0024). This runs as a child process of the Mnema desktop app and drives the
// user's installed PI runtime via its SDK. PI's SDK has shipped under more than one
// npm scope (`@mariozechner/pi-coding-agent`, `@earendil-works/pi-coding-agent`), so
// the shim probes the known scopes and an optional override rather than one fixed name.
//
// PI's builtin coding-agent bash/file tools stay disabled via `noTools: "builtin"`.
// On top of that, this slice registers exactly three custom Mnema broker tools —
// `search`, `timeline`, and `show_text` — whose `execute()` does NOT touch the
// filesystem itself: each call is brokered back to the Rust host over the stdin/stdout
// protocol below, so all capture-history access stays inside Rust's enforcement seam.
//
// Contract (must stay in sync with the Rust integrator):
//   - STDIN:  newline-delimited JSON. The stream STAYS OPEN for the whole session
//             (do NOT assume EOF). Two line kinds:
//               First line (no `type` field):
//                 { "prompt": "<string>" }
//               Subsequent tool-result lines (replies to a tool_call we emitted):
//                 { "type":"tool_result", "id":"<callId>", "ok":true,  "result":<json> }
//                 { "type":"tool_result", "id":"<callId>", "ok":false, "error":"<message>" }
//             The prompt already contains any seeded context; the shim fetches nothing
//             on its own — capture data only flows in through brokered tool_result lines.
//   - ENV:    MNEMA_PI_EXECUTABLE (optional) — absolute path to the user's `pi` binary,
//             used only to resolve the PI SDK package (and `typebox`) when not on
//             NODE_PATH.
//             MNEMA_PI_SDK_PACKAGE (optional) — overrides/extends the SDK package name
//             tried first, for PI builds published under a different npm scope.
//             PI_CODING_AGENT_DIR (optional) — passed through by the parent; honored
//             transparently by AuthStorage.create(), so this shim does not read it.
//   - STDOUT: newline-delimited JSON, exactly one object per line. Protocol only:
//               {"type":"ready"}                                  — session created
//               {"type":"delta","text":"<chunk>"}                 — one answer text_delta
//               {"type":"tool_call","id":"<callId>",              — a custom tool wants the
//                  "tool":"<name>","params":<json>}                  host to run a brokered op
//               {"type":"done"}                                   — answer complete; exit 0
//               {"type":"error","message":"<human>"}              — any failure; exit non-zero
//             Nothing else is ever written to stdout. All diagnostics go to stderr.

import { readFileSync, realpathSync } from "node:fs";
import { dirname, isAbsolute, join } from "node:path";
import { pathToFileURL } from "node:url";
import { createRequire } from "node:module";
import { createInterface } from "node:readline";

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

// ---- stdin: line-based, bidirectional --------------------------------------
//
// The stream stays open for the whole session: the first line carries the prompt,
// and every later line is a brokered tool_result replying to a tool_call we emitted.
// We can't read-to-EOF anymore, so parse line by line with readline.

// Pending tool calls awaiting a host reply: callId -> resolver({ ok, result, error }).
const pendingToolCalls = new Map();

// Resolves with the first prompt string once its line arrives.
let resolvePrompt;
const promptReady = new Promise((resolve) => {
  resolvePrompt = resolve;
});
let promptSeen = false;

/** Wire the readline interface that dispatches every stdin line. */
function startStdinReader() {
  const rl = createInterface({ input: process.stdin });
  rl.on("line", (line) => {
    const trimmed = line.trim();
    if (trimmed.length === 0) return; // ignore blank lines
    let msg;
    try {
      msg = JSON.parse(trimmed);
    } catch (err) {
      diag("ignoring malformed stdin line:", err?.message ?? err);
      return;
    }
    if (!msg || typeof msg !== "object") return;

    if (msg.type === "tool_result") {
      const id = msg.id;
      const resolver = id != null ? pendingToolCalls.get(id) : undefined;
      if (!resolver) {
        diag("tool_result for unknown id:", id);
        return;
      }
      pendingToolCalls.delete(id);
      resolver({
        ok: msg.ok === true,
        result: msg.result,
        error: typeof msg.error === "string" ? msg.error : undefined,
      });
      return;
    }

    if (typeof msg.prompt === "string") {
      // Only the first prompt matters; ignore any later prompt lines.
      if (!promptSeen) {
        promptSeen = true;
        resolvePrompt(msg.prompt);
      }
      return;
    }

    // Anything else is unknown — ignore quietly (diagnostics only).
    diag("ignoring unknown stdin line shape");
  });
  rl.on("error", (err) => {
    finishError(`stdin read error: ${err?.message ?? err}`);
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

/**
 * Resolve and import `typebox` from the SAME node_modules tree as the SDK.
 *
 * Tool `parameters` must be a real TypeBox schema built with the exact `Type`
 * instance the SDK's own tools use — and the SDK imports the bare package name
 * `typebox` (NOT `@sinclair/typebox`). We mirror the SDK resolution strategy so we
 * pick up the same physical module: plain import first, then a require rooted at the
 * real pi binary, then a walk-up for a sibling `node_modules/typebox`.
 *
 * Throws with a clear message if nothing resolves.
 */
async function importTypeBox() {
  // Attempt 1: plain import (NODE_PATH / global modules / same tree as this shim).
  try {
    return await import("typebox");
  } catch (err) {
    diag("plain import of typebox failed:", err?.message ?? err);
  }

  // Attempt 2: resolve relative to the real pi binary, like the SDK.
  const piExe = process.env.MNEMA_PI_EXECUTABLE;
  if (piExe) {
    let realPiPath;
    try {
      realPiPath = realpathSync(piExe);
    } catch (err) {
      diag("could not realpath MNEMA_PI_EXECUTABLE:", err?.message ?? err);
      realPiPath = piExe;
    }

    const candidateUrls = [];

    // 2a: require.resolve("typebox") from the perspective of the real pi binary.
    try {
      const requireFromPi = createRequire(realPiPath);
      candidateUrls.push(requireFromPi.resolve("typebox"));
    } catch (err) {
      diag("createRequire(realPiPath).resolve(typebox) failed:", err?.message ?? err);
    }

    // 2b: walk up from the pi binary dir for a sibling node_modules/typebox,
    // resolving its real ESM entry from package.json.
    {
      let dir = dirname(realPiPath);
      let prev = null;
      while (dir && dir !== prev) {
        const entry = resolvePackageEntry(join(dir, "node_modules", "typebox"));
        if (entry) candidateUrls.push(entry);
        prev = dir;
        dir = dirname(dir);
      }
    }

    for (const candidate of candidateUrls) {
      try {
        const specifier = isAbsolute(candidate)
          ? pathToFileURL(candidate).href
          : candidate;
        return await import(specifier);
      } catch (err) {
        diag("typebox candidate import failed:", candidate, "-", err?.message ?? err);
      }
    }
  }

  throw new Error(
    "Could not resolve the `typebox` package from the PI SDK's module tree. The custom " +
      "Ask AI broker tools need TypeBox to declare their parameter schemas; ensure it is " +
      "installed alongside the PI runtime (co-located with the pi binary).",
  );
}

// ---- custom broker tools ----------------------------------------------------

// Monotonic counter for unique tool-call ids ("t1", "t2", ...).
let toolCallCounter = 0;

/**
 * Broker one tool call back to the Rust host and await its reply.
 *
 * Emits a `tool_call` protocol line, registers a pending resolver keyed by callId,
 * and resolves with `{ content, details }` on `ok:true` — or THROWS on `ok:false`
 * (or on abort), per the SDK's "execute throws on failure" contract.
 */
function callHost(toolName, params, signal) {
  if (signal?.aborted) {
    throw new Error(`${toolName} aborted before dispatch`);
  }
  const id = `t${++toolCallCounter}`;
  return new Promise((resolve, reject) => {
    let settled = false;
    const onAbort = () => {
      if (settled) return;
      settled = true;
      pendingToolCalls.delete(id);
      reject(new Error(`${toolName} aborted`));
    };
    if (signal) signal.addEventListener("abort", onAbort, { once: true });

    pendingToolCalls.set(id, ({ ok, result, error }) => {
      if (settled) return;
      settled = true;
      if (signal) signal.removeEventListener("abort", onAbort);
      if (!ok) {
        reject(new Error(error || `${toolName} tool failed`));
        return;
      }
      resolve({
        content: [{ type: "text", text: JSON.stringify(result) }],
        details: result,
      });
    });

    // Announce the call to the host; the matching tool_result arrives on stdin.
    emit({ type: "tool_call", id, tool: toolName, params });
  });
}

/**
 * Build the three custom Mnema broker tools. `Type` is the SDK-resolved TypeBox
 * factory; `defineTool` is the SDK helper. Each tool's `execute` is a thin broker
 * over `callHost` — the host (Rust) runs the actual brokered capture-history op.
 */
function buildBrokerTools(defineTool, Type) {
  const searchTool = defineTool({
    name: "search",
    label: "Search capture history",
    description:
      "Search the user's redacted on-device capture history (screen OCR + audio " +
      "transcripts). Returns snippets with opaque ids, kinds (screenText/audioTranscript), " +
      "startedAt/endedAt timestamps, and optional context (appName/appBundleId/windowTitle).",
    parameters: Type.Object(
      {
        query: Type.String({ description: "Free-text query to match against captured text." }),
        from: Type.Optional(
          Type.String({ description: "Inclusive lower time bound, RFC3339 (e.g. 2026-06-01T09:00:00Z)." }),
        ),
        to: Type.Optional(
          Type.String({ description: "Inclusive upper time bound, RFC3339." }),
        ),
        limit: Type.Optional(
          Type.Number({ description: "Maximum number of snippets to return." }),
        ),
        app: Type.Optional(
          Type.String({ description: "Restrict to a single app by name or bundle id." }),
        ),
        windowTitle: Type.Optional(
          Type.String({ description: "Restrict to snippets whose window title matches." }),
        ),
      },
      { additionalProperties: false },
    ),
    async execute(_toolCallId, params, signal) {
      return callHost("search", params, signal);
    },
  });

  const timelineTool = defineTool({
    name: "timeline",
    label: "Activity timeline",
    description:
      "Return coarse activity intervals within a bounded time window. Without app/window " +
      "filters the result is audio-oriented; with an app or window title it returns matching " +
      "screen intervals instead.",
    parameters: Type.Object(
      {
        from: Type.String({ description: "Inclusive window start, RFC3339 (required)." }),
        to: Type.String({ description: "Inclusive window end, RFC3339 (required)." }),
        limit: Type.Optional(
          Type.Number({ description: "Maximum number of intervals to return." }),
        ),
        app: Type.Optional(
          Type.String({ description: "Restrict to a single app by name or bundle id." }),
        ),
        windowTitle: Type.Optional(
          Type.String({ description: "Restrict to intervals whose window title matches." }),
        ),
      },
      { additionalProperties: false },
    ),
    async execute(_toolCallId, params, signal) {
      return callHost("timeline", params, signal);
    },
  });

  const showTextTool = defineTool({
    name: "show_text",
    label: "Show derived text",
    description:
      "Return the broker-visible derived text for ONE opaque id previously returned by " +
      "`search`. Use sparingly, only when a snippet is insufficient to answer.",
    parameters: Type.Object(
      {
        opaqueId: Type.String({
          description: "An opaque id from a prior `search` result (required).",
        }),
      },
      { additionalProperties: false },
    ),
    async execute(_toolCallId, params, signal) {
      return callHost("show_text", params, signal);
    },
  });

  return [searchTool, timelineTool, showTextTool];
}

// ---- main ------------------------------------------------------------------

async function main() {
  // 1. Begin reading stdin (line-based). The prompt arrives as the first line;
  //    tool_result lines arrive later. Resolve + import SDK + typebox meanwhile.
  startStdinReader();

  const sdk = await importPiSdk();
  const { AuthStorage, ModelRegistry, createAgentSession, SessionManager, defineTool } = sdk;
  if (!AuthStorage || !ModelRegistry || !createAgentSession || !SessionManager) {
    throw new Error(
      "PI SDK was imported but is missing expected exports " +
        "(AuthStorage, ModelRegistry, createAgentSession, SessionManager).",
    );
  }
  if (typeof defineTool !== "function") {
    throw new Error("PI SDK was imported but is missing the `defineTool` export.");
  }

  const typebox = await importTypeBox();
  const Type = typebox?.Type ?? typebox?.default?.Type;
  if (!Type || typeof Type.Object !== "function") {
    throw new Error("Resolved `typebox` but it is missing the expected `Type` factory.");
  }

  // 2. Wait for the first prompt line.
  const prompt = await promptReady;
  if (typeof prompt !== "string" || prompt.length === 0) {
    throw new Error('stdin must provide a JSON line with a non-empty string "prompt".');
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

  // 4. Create the session. Builtin tools stay disabled; the three custom Mnema
  //    broker tools are registered; the session is ephemeral in-memory.
  const customTools = buildBrokerTools(defineTool, Type);
  const sessionOptions = {
    noTools: "builtin",
    customTools,
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
  //    A tool-enabled agent produces MULTIPLE assistant messages (text, tool call,
  //    more text), so the per-message `done` is NOT terminal — only the top-level
  //    `agent_end` event ends the whole run.
  session.subscribe((event) => {
    if (terminated) return;
    try {
      const type = event?.type;

      // The whole run finished — the real terminal signal.
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
            // One assistant message finished, but more turns may follow (tool
            // calls + further text). Do NOT finish here, or the answer truncates.
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

  // 6. Drive the prompt. The terminal line is normally emitted from `agent_end`,
  //    but guard here too in case the run resolves without a terminal event.
  try {
    await session.prompt(prompt);
  } catch (err) {
    finishError(`PI prompt failed: ${err?.message ?? err}`);
    return;
  }

  // Guarded fallback: a no-op if `agent_end` already finished the run.
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
