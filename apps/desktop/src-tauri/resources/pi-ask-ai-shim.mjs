#!/usr/bin/env node
// pi-ask-ai-shim.mjs
//
// Mnema "Ask AI" PI tool shim — tool-enabled answer slice (GitHub issue #70,
// ADR 0024). This runs as a child process of the Mnema desktop app and drives the
// user's installed PI runtime via its SDK. PI's SDK has shipped under more than one
// npm scope (`@mariozechner/pi-coding-agent`, `@earendil-works/pi-coding-agent`), so
// the shim probes the known scopes and an optional override rather than one fixed name.
//
// PI's builtin coding-agent bash/file tools stay disabled via `noTools: "all"` plus an
// explicit `tools` allowlist of ONLY the Mnema broker tool names, and the user's
// extensions/skills/prompt-templates/context files are kept out entirely by passing an
// explicitly empty `resourceLoader` (a `DefaultResourceLoader` constructed with
// `noExtensions/noSkills/noPromptTemplates/noThemes/noContextFiles`). Omitting the
// resource loader would let `createAgentSession` build the DefaultResourceLoader and load
// the user's configured extensions/skills/context — whose hooks and prompt context would
// then run inside an Ask AI session carrying seeded capture history — so we never omit it.
// On top of that, this slice registers a small set of custom Mnema broker tools —
// `search`, `timeline`, `show_text`, and the presentation-only `reference_captures` —
// whose `execute()` does NOT touch the filesystem itself: each call is brokered back to
// the Rust host over the stdin/stdout protocol below, so all capture-history access
// stays inside Rust's enforcement seam.
//
// Contract (must stay in sync with the Rust integrator):
//   - STDIN:  newline-delimited JSON. The stream STAYS OPEN for the whole session
//             (do NOT assume EOF). The session is MULTI-TURN: each `prompt` line
//             starts a new turn on the SAME live PI session, which retains prior
//             turns' conversation history in-memory. Three line kinds:
//               Prompt lines (no `type` field) — one per turn, in arrival order:
//                 { "prompt": "<string>" }
//               Tool-result lines (replies to a tool_call we emitted):
//                 { "type":"tool_result", "id":"<callId>", "ok":true,  "result":<json> }
//                 { "type":"tool_result", "id":"<callId>", "ok":false, "error":"<message>" }
//             Prompts are queued and answered one at a time; a new `prompt` line that
//             arrives while a turn is running starts the next turn once the current one
//             finishes. The session ends when stdin closes (EOF) and the queue is
//             drained, or when the child is killed. The prompt already contains any
//             seeded context; the shim fetches nothing on its own — capture data only
//             flows in through brokered tool_result lines.
//   - ENV:    MNEMA_PI_EXECUTABLE (optional) — absolute path to the user's `pi` binary,
//             used only to resolve the PI SDK package (and `typebox`) when not on
//             NODE_PATH.
//             MNEMA_PI_SDK_PACKAGE (optional) — overrides/extends the SDK package name
//             tried first, for PI builds published under a different npm scope.
//             MNEMA_PI_ASK_AI_MODEL (optional) — "provider:id" of the model Quick
//             Recall should use; falls back to the PI default when unset/not found.
//             MNEMA_PI_LIST_MODELS (optional) — when "1", run in list mode: build the
//             model registry, emit one {"type":"models","models":[...]} line, exit 0.
//             No prompt is read and no tools are registered in this mode.
//             PI_CODING_AGENT_DIR (optional) — passed through by the parent; honored
//             transparently by AuthStorage.create(), so this shim does not read it.
//   - STDOUT: newline-delimited JSON, exactly one object per line. Protocol only:
//               {"type":"ready"}                                  — session created
//               {"type":"delta","text":"<chunk>"}                 — one answer text_delta
//                                                                    (belongs to the
//                                                                    current turn)
//               {"type":"tool_call","id":"<callId>",              — a custom tool wants the
//                  "tool":"<name>","params":<json>}                  host to run a brokered op
//               {"type":"done"}                                   — the CURRENT TURN
//                                                                    finished; the process
//                                                                    KEEPS RUNNING for the
//                                                                    next prompt (repeatable)
//               {"type":"error","message":"<human>"}              — FATAL failure only; then
//                                                                    exit non-zero
//             The process exits 0 (no `done`) when stdin closes and no prompt is pending.
//             Nothing else is ever written to stdout. All diagnostics go to stderr.

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
//
// Two distinct guards now that the session is multi-turn:
//   - `ended` is the FATAL / process-winding-down guard. Once a fatal error fires
//     (or the process is otherwise shutting down) no further protocol output —
//     `delta`, per-turn `done`, or another `error` — is allowed. It is set only by
//     `finishError` and the clean-EOF exit path.
//   - per-turn "done already emitted" guarding lives on each turn's state
//     (`turn.settled` in the main loop), so a turn's `agent_end` and the loop's own
//     `done` cannot double-emit, while a NEW turn can still emit its own `done`.
let ended = false;

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
  if (ended) return;
  emit({ type: "delta", text });
}

/**
 * Emit a turn-level `done` at most once for the given turn, WITHOUT exiting the
 * process — the session stays alive for the next prompt. `turn.settled` is the
 * per-turn guard so a turn's `agent_end` and the main loop's own completion can
 * each call this but only one `done` line is written for that turn.
 */
function finishTurn(turn) {
  if (ended || !turn || turn.settled) return;
  turn.settled = true;
  emit({ type: "done" });
}

/**
 * Emit the terminal error line at most once and exit non-zero. This is the only
 * path that ends the whole process abnormally; it sets `ended` so no further
 * protocol output escapes after a fatal failure.
 */
function finishError(message) {
  if (ended) return;
  ended = true;
  emit({ type: "error", message: String(message) });
  process.exit(1);
}

/** Clean shutdown: stdin closed with no pending prompts — exit 0 with no `done`. */
function finishSession() {
  if (ended) return;
  ended = true;
  process.exit(0);
}

// ---- stdin: line-based, bidirectional --------------------------------------
//
// The stream stays open for the whole multi-turn session. Three line kinds flow in:
// prompt lines (one per turn), tool_result lines (replies to a tool_call we emitted),
// and EOF (stdin close). We can't read-to-EOF for a single answer anymore, so parse
// line by line and feed prompts through a queue the main loop drains.
//
// Framing splits on literal LF (\n) bytes ONLY — NOT via readline. The inbound JSON
// lines are produced by `serde_json` on the Rust side, which (like JSON.stringify)
// leaves the Unicode line separators U+2028 / U+2029 RAW (unescaped) inside string
// values. Node's `readline` treats U+2028 / U+2029 as line terminators, so a prompt
// or tool_result carrying copied/captured text with those code points would be split
// mid-JSON, fail JSON.parse, and be dropped — silently failing the ask or leaving a
// tool call unresolved. Splitting on \n only keeps each serde_json line intact.

// Pending tool calls awaiting a host reply: callId -> resolver({ ok, result, error }).
const pendingToolCalls = new Map();

// FIFO queue of prompt strings the main loop has not yet answered. The loop pulls
// one at a time; new prompt lines push here while a turn is in flight.
const promptQueue = [];
// True once stdin has closed (EOF): no more prompts will ever arrive.
let stdinClosed = false;
// A re-armable notifier the main loop awaits when the queue is empty. Resolving it
// wakes the loop so it can pull a freshly-queued prompt or observe EOF.
let notifyPromptWaiter = null;

/** Wake any loop blocked in `nextPrompt()` so it re-checks the queue / EOF flag. */
function signalPromptWaiter() {
  if (notifyPromptWaiter) {
    const resolve = notifyPromptWaiter;
    notifyPromptWaiter = null;
    resolve();
  }
}

/**
 * Pull the next prompt for the main loop. Resolves with the prompt string when one
 * is available, or with `null` when stdin has closed and the queue is drained (the
 * loop's clean-exit signal).
 */
async function nextPrompt() {
  for (;;) {
    if (promptQueue.length > 0) return promptQueue.shift();
    if (stdinClosed) return null;
    // Re-arm the waiter and block until a prompt arrives or stdin closes.
    await new Promise((resolve) => {
      notifyPromptWaiter = resolve;
    });
  }
}

/** Dispatch one already-split stdin line (no trailing LF). */
function dispatchStdinLine(line) {
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
    // Each prompt line starts another turn: enqueue it and wake the main loop.
    promptQueue.push(msg.prompt);
    signalPromptWaiter();
    return;
  }

  // Anything else is unknown — ignore quietly (diagnostics only).
  diag("ignoring unknown stdin line shape");
}

/**
 * Read stdin and dispatch every LF-delimited line. Frames on literal LF (\n)
 * bytes ONLY (not readline), so raw U+2028 / U+2029 inside a serde_json string
 * value can never split a line mid-JSON. Partial lines buffer across chunk
 * boundaries, and a trailing \r is stripped for Windows CRLF safety.
 */
function startStdinReader() {
  process.stdin.setEncoding("utf8");
  let buffer = "";
  process.stdin.on("data", (chunk) => {
    buffer += chunk;
    let newlineIndex;
    while ((newlineIndex = buffer.indexOf("\n")) !== -1) {
      let line = buffer.slice(0, newlineIndex);
      buffer = buffer.slice(newlineIndex + 1);
      // Strip a trailing \r (Windows CRLF) before dispatching.
      if (line.endsWith("\r")) line = line.slice(0, -1);
      dispatchStdinLine(line);
    }
  });
  process.stdin.on("end", () => {
    // Flush any final line that arrived without a trailing newline.
    if (buffer.length > 0) {
      let line = buffer;
      buffer = "";
      if (line.endsWith("\r")) line = line.slice(0, -1);
      dispatchStdinLine(line);
    }
    // EOF: no further prompts. Wake a waiting loop so it can drain + exit cleanly.
    stdinClosed = true;
    signalPromptWaiter();
  });
  process.stdin.on("error", (err) => {
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
 * Build the custom Mnema broker tools. `Type` is the SDK-resolved TypeBox
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

  // `reference_captures` is a PRESENTATION signal, not a data tool: it nominates the
  // captures (screen frames / audio) that back the answer so the app can surface them
  // to the user as source cards. It returns NO capture data — only a small ack of how
  // many ids were accepted/dropped. The model passes the opaque ids it received from
  // `search` results, ordered most-relevant-first, and calls this once near the end of
  // the answer (a repeat call replaces the prior set). It does NOT count against the
  // tool-call budget.
  const referenceCapturesTool = defineTool({
    name: "reference_captures",
    label: "Reference source captures",
    description:
      "Presentation signal that nominates the captures (screen frames / audio) behind " +
      "your answer so the app can show them to the user as source cards. Returns NO " +
      "capture data — only an acknowledgement of how many were accepted/dropped. Pass " +
      "the opaque ids you received from `search` results, ordered most-relevant-first, " +
      "and call this once near the end of your answer (a repeat call replaces the prior " +
      "set). This does NOT count against the tool-call budget.",
    parameters: Type.Object(
      {
        opaqueIds: Type.Array(
          Type.String({ description: "An opaque id from a prior search result." }),
          { description: "Opaque ids of the captures behind the answer, most-relevant-first." },
        ),
      },
      { additionalProperties: false },
    ),
    async execute(_toolCallId, params, signal) {
      return callHost("reference_captures", params, signal);
    },
  });

  // `recall_context` returns ONLY the User-Context conclusions/activities relevant
  // to the question — never the whole dossier, never sensitive-category
  // conclusions. It is the fast way to answer "what do you know about me / my
  // habits / my projects" without raking raw captures.
  const recallContextTool = defineTool({
    name: "recall_context",
    label: "Recall user context",
    description:
      "Return ONLY the User-Context conclusions (distilled beliefs about the user) and recent " +
      "activities that are relevant to the question. Redacted and capped — it NEVER returns the " +
      "whole dossier and NEVER returns sensitive-category conclusions. Use this for questions " +
      "about the user's habits, interests, projects, or what you know about them, instead of " +
      "raw `search`.",
    parameters: Type.Object(
      {
        query: Type.String({
          description:
            "The user's question; returns only the User-Context conclusions/activities relevant to it.",
        }),
        limit: Type.Optional(
          Type.Number({ description: "Maximum number of conclusions/activities to return (capped server-side)." }),
        ),
      },
      { additionalProperties: false },
    ),
    async execute(_toolCallId, params, signal) {
      return callHost("recall_context", params, signal);
    },
  });

  return [searchTool, timelineTool, showTextTool, recallContextTool, referenceCapturesTool];
}

// ---- model registry helpers -------------------------------------------------

/** Stable `provider:id` value used to persist + reselect a model. */
function modelValue(model) {
  if (!model || typeof model !== "object") return null;
  const provider = typeof model.provider === "string" ? model.provider : null;
  const id = typeof model.id === "string" ? model.id : null;
  if (!provider || !id) return null;
  return `${provider}:${id}`;
}

/**
 * Enumerate selectable models from the registry. Prefers models that already
 * have auth configured (`getAvailable`); falls back to all known models so the
 * picker is never empty just because auth probing varies across PI releases.
 *
 * `getAvailable`/`getAll` may be sync or return a Promise depending on the PI
 * release, so both are awaited; awaiting a non-Promise is a no-op. Without the
 * await a Promise-returning `getAvailable` would fail the `Array.isArray` check
 * and silently fall through to `getAll`, listing every known model instead of
 * only the authenticated ones.
 */
async function collectModels(modelRegistry) {
  let models = [];
  try {
    if (typeof modelRegistry.getAvailable === "function") {
      models = (await modelRegistry.getAvailable()) ?? [];
    }
  } catch (err) {
    diag("getAvailable failed:", err?.message ?? err);
  }
  if (!Array.isArray(models) || models.length === 0) {
    try {
      if (typeof modelRegistry.getAll === "function") {
        models = (await modelRegistry.getAll()) ?? [];
      }
    } catch (err) {
      diag("getAll failed:", err?.message ?? err);
    }
  }
  const seen = new Set();
  const out = [];
  for (const model of Array.isArray(models) ? models : []) {
    const value = modelValue(model);
    if (!value || seen.has(value)) continue;
    seen.add(value);
    out.push({
      value,
      provider: model.provider,
      id: model.id,
      name: typeof model.name === "string" ? model.name : model.id,
    });
  }
  return out;
}

/**
 * Resolve the `provider:id` spec from MNEMA_PI_ASK_AI_MODEL into a Model via the
 * registry. Returns undefined when unset, malformed, or not found so the caller
 * falls back to the PI default.
 */
function resolveSelectedModel(modelRegistry, spec) {
  if (typeof spec !== "string") return undefined;
  const trimmed = spec.trim();
  if (trimmed.length === 0) return undefined;
  const sep = trimmed.indexOf(":");
  if (sep <= 0 || sep >= trimmed.length - 1) {
    diag("ignoring malformed MNEMA_PI_ASK_AI_MODEL:", trimmed);
    return undefined;
  }
  const provider = trimmed.slice(0, sep);
  const id = trimmed.slice(sep + 1);
  try {
    if (typeof modelRegistry.find === "function") {
      const found = modelRegistry.find(provider, id);
      if (found) return found;
    }
  } catch (err) {
    diag("model find failed:", err?.message ?? err);
  }
  diag(`configured Ask AI model "${trimmed}" not found in registry; using default`);
  return undefined;
}

// ---- main ------------------------------------------------------------------

async function main() {
  // List mode (MNEMA_PI_LIST_MODELS=1) is a separate, prompt-less invocation:
  // it builds the registry, emits one `{"type":"models",...}` line, and exits.
  // It never reads a prompt or registers tools.
  const listMode = process.env.MNEMA_PI_LIST_MODELS === "1";

  // 1. Begin reading stdin (line-based) for an interactive answer session. The
  //    prompt arrives as the first line; tool_result lines arrive later. Resolve
  //    + import SDK + typebox meanwhile. List mode needs no stdin.
  if (!listMode) startStdinReader();

  const sdk = await importPiSdk();
  const {
    AuthStorage,
    ModelRegistry,
    createAgentSession,
    SessionManager,
    defineTool,
    DefaultResourceLoader,
    getAgentDir,
  } = sdk;
  if (!AuthStorage || !ModelRegistry || !createAgentSession || !SessionManager) {
    throw new Error(
      "PI SDK was imported but is missing expected exports " +
        "(AuthStorage, ModelRegistry, createAgentSession, SessionManager).",
    );
  }

  // Build auth + model registry from the user's existing PI config.
  const authStorage = AuthStorage.create();
  const modelRegistry = ModelRegistry.create(authStorage);

  // List mode: enumerate selectable models, emit one line, and exit.
  if (listMode) {
    emit({ type: "models", models: await collectModels(modelRegistry) });
    process.exit(0);
    return;
  }

  if (typeof defineTool !== "function") {
    throw new Error("PI SDK was imported but is missing the `defineTool` export.");
  }

  const typebox = await importTypeBox();
  const Type = typebox?.Type ?? typebox?.default?.Type;
  if (!Type || typeof Type.Object !== "function") {
    throw new Error("Resolved `typebox` but it is missing the expected `Type` factory.");
  }

  // 2. Resolve the user's configured default model, if the registry exposes one.
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

  // A user-selected Quick Recall model (MNEMA_PI_ASK_AI_MODEL, "provider:id")
  // overrides the PI default when it resolves; otherwise fall back to default.
  const selectedModel = resolveSelectedModel(
    modelRegistry,
    process.env.MNEMA_PI_ASK_AI_MODEL,
  );
  const chosenModel = selectedModel ?? defaultModel;

  // 4. Create the session. Only the Mnema broker tools are reachable; the session
  //    is ephemeral in-memory.
  const customTools = buildBrokerTools(defineTool, Type);
  // Restrict the session to EXACTLY the Mnema broker tools via an explicit
  // allowlist. `noTools: "builtin"` is NOT sufficient: per the PI SDK it only
  // disables the builtin coding tools (read/bash/edit/write) and KEEPS any
  // user-installed extension tools enabled, which would leak arbitrary machine
  // tools into Ask AI and violate the shim's "no file/bash/other tools" contract.
  // The SDK applies the `tools` allowlist uniformly to builtin, extension, AND
  // custom tools, so listing only our tool names drops builtins + extensions while
  // our custom tools (matched by name) survive the same filter.
  const allowedToolNames = customTools
    .map((tool) => tool?.name)
    .filter((name) => typeof name === "string" && name.length > 0);

  // The tools allowlist is NOT enough on its own. When `resourceLoader` is omitted,
  // `createAgentSession` builds a `DefaultResourceLoader` and `await`s its `reload()`,
  // which discovers and LOADS the user's configured extensions, skills, prompt
  // templates, and context (AGENTS) files. The allowlist only filters which *tools*
  // are active; loaded extensions still get their hooks wired (e.g.
  // `before_provider_request`, `context`, `before_agent_start`) and their skills /
  // system-prompt / context files injected into the model context. That extension
  // code/prompt context would run inside an Ask AI session carrying seeded capture
  // history. So we pass an explicitly empty resource loader: no extensions, skills,
  // prompt templates, themes, or context files are loaded for Ask AI.
  let resourceLoader;
  if (typeof DefaultResourceLoader === "function") {
    try {
      const agentDir = typeof getAgentDir === "function" ? getAgentDir() : undefined;
      const loader = new DefaultResourceLoader({
        cwd: process.cwd(),
        agentDir,
        noExtensions: true,
        noSkills: true,
        noPromptTemplates: true,
        noThemes: true,
        noContextFiles: true,
      });
      await loader.reload();
      resourceLoader = loader;
    } catch (err) {
      // If we cannot build the disabled loader, fail closed rather than silently
      // falling back to the DefaultResourceLoader that loads user extensions.
      throw new Error(
        `Failed to build the disabled Ask AI resource loader (refusing to fall back ` +
          `to user extensions): ${err?.message ?? err}`,
      );
    }
  } else {
    throw new Error(
      "PI SDK is missing the `DefaultResourceLoader` export needed to disable " +
        "extension/skill/context loading for Ask AI.",
    );
  }

  const sessionOptions = {
    // Belt-and-suspenders: start from no tools, then allowlist only ours.
    noTools: "all",
    tools: allowedToolNames,
    customTools,
    // Empty resource loader: never load the user's extensions/skills/context.
    resourceLoader,
    authStorage,
    modelRegistry,
    sessionManager: SessionManager.inMemory(),
  };
  if (chosenModel) {
    sessionOptions.model = chosenModel;
  }

  let session;
  try {
    const created = await createAgentSession(sessionOptions);
    session = created?.session ?? created;
  } catch (err) {
    const msg = String(err?.message ?? err);
    // A common failure here is no configured provider/model.
    if (/model|provider|auth|credential/i.test(msg) && !chosenModel) {
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

  // The turn currently being answered. Each entry is `{ settled }`:
  //   - `settled` is the per-turn "done already emitted" guard.
  // `currentTurn` is the live turn the subscribe handler routes events to; the main
  // loop swaps in a fresh turn before each `session.prompt()`.
  let currentTurn = null;
  // Per-turn completion promise the loop awaits. `agent_end` resolves it for the
  // active turn; the loop re-arms it at the start of each turn.
  let resolveTurnComplete = null;

  // 5. Subscribe ONCE, BEFORE the first prompt, so we never miss early streamed
  //    tokens. The single subscription serves every turn for the life of the
  //    session (the SDK retains conversation history across `session.prompt()`
  //    calls). A tool-enabled agent produces MULTIPLE assistant messages per turn
  //    (text, tool call, more text), so the per-message `done` is NOT terminal —
  //    only the top-level `agent_end` ends the CURRENT TURN (not the process).
  session.subscribe((event) => {
    if (ended) return;
    try {
      const type = event?.type;

      // The current turn finished. Emit this turn's `done` and let the main loop
      // advance to the next prompt — the process keeps running.
      if (type === "agent_end") {
        const turn = currentTurn;
        finishTurn(turn);
        if (resolveTurnComplete) {
          const resolve = resolveTurnComplete;
          resolveTurnComplete = null;
          resolve();
        }
        return;
      }

      // Streaming message updates carry the assistant message sub-events. With the
      // session retaining history, deltas belong to whichever turn is running now.
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
            // An assistant-message error is fatal to the whole session.
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

  // 6. Multi-turn main loop. Pull prompts one at a time; each prompt is a turn on
  //    the same live session. The loop exits cleanly (0) when stdin closes and no
  //    prompt is pending; a fatal failure exits non-zero via `finishError`.
  for (;;) {
    const prompt = await nextPrompt();
    if (prompt === null) {
      // stdin closed and the queue is drained: end the session cleanly.
      finishSession();
      return;
    }
    if (typeof prompt !== "string" || prompt.length === 0) {
      throw new Error('stdin must provide a JSON line with a non-empty string "prompt".');
    }

    // Start a fresh turn and arm its completion promise BEFORE prompting, so an
    // `agent_end` that arrives during `session.prompt()` resolves the right turn.
    const turn = { settled: false };
    currentTurn = turn;
    const turnComplete = new Promise((resolve) => {
      resolveTurnComplete = resolve;
    });

    try {
      await session.prompt(prompt);
    } catch (err) {
      finishError(`PI prompt failed: ${err?.message ?? err}`);
      return;
    }

    // `session.prompt()` may resolve before or after `agent_end`. Resolve the turn
    // here too so the loop never hangs if `agent_end` never arrives for this turn;
    // if `agent_end` already resolved it, this is a no-op (`resolveTurnComplete` was
    // cleared). Then await the turn's completion and, if a fatal error fired
    // mid-turn, bail out.
    if (resolveTurnComplete) {
      const resolve = resolveTurnComplete;
      resolveTurnComplete = null;
      resolve();
    }
    await turnComplete;
    if (ended) return;
    // `finishTurn` is idempotent per turn, so emitting the turn's `done` here is a
    // guarded fallback when no `agent_end` produced it.
    finishTurn(turn);
  }
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
