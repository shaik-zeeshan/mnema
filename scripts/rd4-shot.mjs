// Round-4 direction-05 render harness.
//
// The ONLY sanctioned way to verify this branch: screenshot the real app in a
// real browser and LOOK at the PNG. Grepping class names has produced false
// verdicts on this repo before.
//
//   bun x playwright install chromium          # once
//   nohup bun --cwd=apps/desktop run dev > /tmp/vite-05.log 2>&1 & disown
//   node scripts/rd4-shot.mjs /overview dark 1100x720 docs/redesign/round4/05-tactile-instruments/shots
//
// Args: <route> <theme: light|dark> <WxH> <outDir> [portFromViteLog]
// The port auto-bumps if 1420 is taken — read /tmp/vite-05.log for the real one
// and pass it via the PORT env var.

import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const [route = "/", theme = "dark", size = "1100x720", outDir = "/tmp/rd4-shots"] = process.argv.slice(2);
const port = process.env.PORT ?? "1420";
const [width, height] = size.split("x").map(Number);

// Tauri IPC stub. Rule: NEVER return bare `null` for an unmocked command — a
// null freezes the Svelte tree and every subsequent screenshot is a lie.
const initScript = `
// ── Timeline fixture (additive) ──────────────────────────────────────────────
// Enough shape for the rail to draw app bands, the lane to draw mic/sys bars,
// the jump menu to have days you can land on, and the drawer to open with a
// waveform + turns. Every number here is fixture data, never copy.
const TL_NOW = Date.now();
// 2 s apart = the shipping default rate. It must stay under the app-run split
// threshold (timelineGapMs, 10 s floor) or every frame becomes its own run and
// the rail draws 149 one-frame bands instead of app bands.
const TL_STEP = 2000;
const TL_APPS = [
  ["com.microsoft.VSCode", "Code", "answer_view.rs — mnema", 34],
  ["com.apple.Safari", "Safari", "docs.polar.sh — webhook signatures", 22],
  ["com.tinyspeck.slackmacgap", "Slack", "#launch — 3 unread", 18],
  ["com.figma.Desktop", "Figma", "Round 4 — timeline", 12],
  ["md.obsidian", "Obsidian", "Mnema.md", 15],
  ["com.apple.Terminal", "Terminal", "cargo test -p app-infra", 9],
];
const TL_FRAMES = (() => {
  const out = [];
  let id = 4000;
  let t = TL_NOW - 2000;
  let a = 0;
  while (out.length < 260) {
    const [bundleId, appName, windowTitle, run] = TL_APPS[a % TL_APPS.length];
    for (let k = 0; k < run && out.length < 260; k++) {
      out.push({
        id: id--, sessionId: "sess-fixture", filePath: "/fixture/frame.png",
        capturedAt: new Date(t).toISOString(), width: 2560, height: 1440,
        appBundleId: bundleId, appName, windowTitle,
        url: null, ocrText: null, processorVersion: null, equivalenceHint: null,
        createdAt: new Date(t).toISOString(), updatedAt: new Date(t).toISOString(),
      });
      t -= TL_STEP;
    }
    a++;
  }
  return out;                                // newest first, as the backend returns
})();
// A CSS-free stand-in screenshot: a dark editor-ish SVG, so the stage has a real
// 16:9 image to float instead of a grey rectangle.
const TL_SHOT = "data:image/svg+xml;base64," + btoa(
  '<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720">' +
  '<rect width="1280" height="720" fill="#15151d"/>' +
  '<rect width="70" height="720" fill="#0f0f16"/><rect x="70" width="250" height="720" fill="#15151d"/>' +
  '<rect x="320" width="960" height="46" fill="#0f0f16"/>' +
  Array.from({ length: 22 }, (_, i) =>
    '<rect x="' + (360 + (i % 3) * 24) + '" y="' + (76 + i * 27) + '" width="' +
    (160 + ((i * 97) % 520)) + '" height="10" rx="2" fill="' +
    ["#4c4c62", "#c08cf0", "#8fd694", "#6fb8f0", "#48584f"][i % 5] + '"/>').join("") +
  Array.from({ length: 12 }, (_, i) =>
    '<rect x="96" y="' + (80 + i * 34) + '" width="' + (90 + ((i * 53) % 150)) +
    '" height="9" rx="2" fill="#33334a"/>').join("") +
  '</svg>');
// Offsets are MINUTES before now. They must land inside the RAIL'S VISIBLE
// SPAN (~2 min at 8 px/frame), not merely inside the loaded window, or the
// lane looks empty; the last one straddles now so the active frame has an
// audio moment (that is what the P shortcut opens).
const TL_SEGS = [
  ["microphone", 1, -2.3, -1.6], ["systemAudio", 1, -2.3, -1.6],
  ["microphone", 2, -1.1, -0.4], ["systemAudio", 2, -3.0, -2.6],
  ["systemAudio", 3, -0.9, 0.05],
].map(([sourceKind, segmentIndex, fromMin, toMin], i) => ({
  id: 900 + i, sourceKind, sourceSessionId: "sess-fixture", segmentIndex,
  filePath: "/fixture/" + sourceKind + "-" + segmentIndex + ".m4a",
  startedAt: new Date(TL_NOW + fromMin * 60000).toISOString(),
  endedAt: new Date(TL_NOW + toMin * 60000).toISOString(),
  createdAt: new Date(TL_NOW).toISOString(), updatedAt: new Date(TL_NOW).toISOString(),
}));
const TL_COVERAGE = (() => {
  const out = [];
  for (let back = 0; back < 34; back++) {
    if (back === 4 || back === 9 || back === 10 || back === 17) continue;   // empty days
    const d = new Date(TL_NOW - back * 86400000);
    const key = d.getFullYear() + "-" + String(d.getMonth() + 1).padStart(2, "0") +
      "-" + String(d.getDate()).padStart(2, "0");
    // Today's hours must start at 0: the hour list caps today at the CURRENT
    // hour, so a fixture that only covers 8am+ reads "no frames" all morning.
    const first = back === 0 ? 0 : 8 + (back % 3);
    const last = first + 3 + ((back * 5) % 7);
    const hours = [];
    for (let h = first; h <= Math.min(22, last); h++) hours.push(h);
    out.push({ day: key, coveredMs: hours.length * 2400000, hours });
  }
  return out;
})();
// Month summaries feed the jump menu's hour list; without them every previewed
// day reads "No frames on this day". One id per covered hour is enough.
const TL_SUMMARIES = (range) => {
  const s = new Date(range.capturedAtStart).getTime();
  const e = new Date(range.capturedAtEnd).getTime();
  const out = [];
  let id = 1;
  for (const day of TL_COVERAGE) {
    for (const h of day.hours) {
      const [y, m, d] = day.day.split("-").map(Number);
      const at = new Date(y, m - 1, d, h, 12).getTime();
      if (at < s || at > e) continue;
      for (let k = 0; k < 3 + (h % 5); k++) out.push({ id: id++, capturedAt: new Date(at + k * 60000).toISOString() });
    }
  }
  return out;
};
// OCR observations for the active frame, in the payload's normalized space
// (origin bottom-left). Enough regions for the overlay to have something real.
const TL_OCR = Array.from({ length: 14 }, (_, i) => ({
  text: ["fn verify_signature", "let raw = secret.as_bytes()", "// hash the raw whsec_ string",
    "assert_eq!(sig, expected)", "webhook.rs", "cargo test -p licensegate"][i % 6],
  confidence: 0.82 + (i % 7) * 0.02,
  boundingBox: { x: 0.28 + (i % 3) * 0.02, y: 0.86 - i * 0.037, width: 0.12 + ((i * 7) % 30) / 100, height: 0.014 },
}));
// A real (silent) WAV so the drawer's <audio> loads instead of erroring.
const TL_WAV = (() => {
  const n = 8000, b = new Uint8Array(44 + n * 2), dv = new DataView(b.buffer);
  const put = (o, s) => { for (let i = 0; i < s.length; i++) b[o + i] = s.charCodeAt(i); };
  put(0, "RIFF"); dv.setUint32(4, 36 + n * 2, true); put(8, "WAVEfmt ");
  dv.setUint32(16, 16, true); dv.setUint16(20, 1, true); dv.setUint16(22, 1, true);
  dv.setUint32(24, 8000, true); dv.setUint32(28, 16000, true);
  dv.setUint16(32, 2, true); dv.setUint16(34, 16, true);
  put(36, "data"); dv.setUint32(40, n * 2, true);
  let s = ""; for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i]);
  return btoa(s);
})();
const TL_TURNS = [
  [1, "Speaker 1", 6000, 21000, "The webhook is still 400-ing on the raw secret — I think we are hashing the decoded bytes."],
  [2, "Speaker 2", 22000, 38000, "That would explain the fixtures passing locally. I will check webhook.rs after this."],
  [3, "Speaker 3", 39000, 52000, "Can we get delivery ids into the log line while you are in there?"],
  [1, "Speaker 1", 53000, 74000, "Yes — and the retry count, otherwise the dashboard cannot tell a flake from a real failure."],
].map(([clusterId, speakerLabel, startMs, endMs, transcriptText], i) => ({
  id: 700 + i, audioSegmentId: 900, sessionId: "sess-fixture", clusterId,
  segmentClusterId: clusterId, providerClusterId: "p" + clusterId, speakerLabel,
  personId: null, personLinkAuto: false, suggestedPersonId: null,
  recognitionConfidence: null, recognitionScore: null,
  startMs, endMs, transcriptText, overlaps: false,
}));

// ── Overview bento fixture (additive) ───────────────────────────────────────
// Enough for the tiles to have something to read: headline frames, the day's
// conversations, a digest that carries its own open-thread sentence (G11), the
// context counts, and REAL-SHAPED system facts so the day-budget gauge can
// compute a denominator. Set EMPTY=1 to shoot the fresh-machine face instead —
// every read below then answers empty, exactly as day one does.
const OV_EMPTY = ${process.env.EMPTY === "1"};
// SEM=on  → semantic search enabled with a part-built index (the coverage meter)
// SEM=off → disabled but with a real un-indexed count (the price-before-enable
//           ghost gauge, G10). Unset keeps the shipping default: both counts
//           null, which is what prod ships (zero embeddings).
const SEM = "${process.env.SEM ?? ""}";
const SEM_FACTS = SEM === "on"
  ? { semanticVectorCount: 412000, semanticPendingCount: 248000 }
  : SEM === "off"
    ? { semanticVectorCount: 0, semanticPendingCount: 1240000 }
    : {};
const OV_SHOT = (fill, ink) => "data:image/svg+xml;base64," + btoa(
  '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="300">' +
  '<rect width="480" height="300" fill="' + fill + '"/>' +
  Array.from({ length: 10 }, (_, i) =>
    '<rect x="' + (40 + (i % 2) * 18) + '" y="' + (36 + i * 24) + '" width="' +
    (110 + ((i * 71) % 260)) + '" height="8" rx="2" fill="' + ink + '"/>').join("") +
  '</svg>');
const OV_MOMENTS = [
  ["Obsidian", "#1d1d24", "#4c4c62", 9.2], ["Figma", "#2c2c33", "#a259ff", 10.7],
  ["Zoom", "#16161c", "#5b8def", 13.0], ["Terminal", "#0c0e0d", "#4ade80", 13.8],
  ["Code", "#1b1b24", "#6fb8f0", 14.5],
].map(([title, fill, ink, hour], i) => {
  const at = new Date(TL_NOW); at.setHours(Math.floor(hour), (hour % 1) * 60, 0, 0);
  return {
    frameId: 5000 + i, filePath: OV_SHOT(fill, ink), capturedAtMs: at.getTime(),
    activityId: 60 + i, title, focus: null,
    activityStartedAtMs: at.getTime() - 1800000, activityEndedAtMs: at.getTime() + 1800000,
    durationMs: 3600000,
  };
});
const OV_CONVERSATIONS = [
  ["Launch sync", 13.03, 2280000, 5], ["Design review", 10.2, 2460000, 3],
  ["1:1 with Priya", 9.5, 1320000, 2],
].map(([title, hour, spokenMs, speakerCount], i) => {
  const at = new Date(TL_NOW); at.setHours(Math.floor(hour), Math.round((hour % 1) * 100), 0, 0);
  return {
    activityId: 70 + i, title, startedAtMs: at.getTime(),
    endedAtMs: at.getTime() + spokenMs, spokenMs, speakerCount,
  };
});
const OV_ASKS = [
  ["What did Priya say about the webhook?", 4, 13.58],
  ["Where did the afternoon go?", 2, 15.2],
].map(([title, turnCount, hour], i) => {
  const at = new Date(TL_NOW); at.setHours(Math.floor(hour), Math.round((hour % 1) * 100), 0, 0);
  return {
    conversationId: "conv-" + i, title, origin: "quick_recall",
    createdAtMs: at.getTime(), updatedAtMs: at.getTime(), turnCount, preview: title,
  };
});
const OV_DIGEST = {
  rangeKind: "day", rangeStartMs: TL_NOW - 43200000, rangeEndMs: TL_NOW,
  headline: "A licensing day that ended in the webhook log.",
  narrative: "Mostly licensing: the Polar webhook fix landed after the launch sync, and the " +
    "afternoon went to answer_view.rs and the round-4 mockups. One open thread — Tom wants " +
    "delivery ids in the webhook log line before the dashboard can tell a flake from a real failure.",
  generatedAtMs: TL_NOW - 1500000,
};
const OV_FACTS = {
  capturePath: "/Users/you/Movies/Mnema", diskFreeBytes: 214000000000,
  totalRamBytes: 17179869184, measuredBytesPerDay: 1350000000, measuredDays: 7,
  screenFrameRate: 0.5, ocrBacklog: 0, transcriptionBacklog: 0,
  semanticVectorCount: null, semanticPendingCount: null, semanticVectorBytes: 768,
  databaseBytes: 2400000000,
};
const OV_USAGE = {
  rangeStartMs: TL_NOW - 43200000, rangeEndMs: TL_NOW, timePerSite: [],
  appTransitions: [], activityHeatmap: [],
  timePerApp: TL_APPS.map(([appBundleId, app], i) => ({
    app, appBundleId, activeMs: (6 - i) * 1800000, frameCount: 12000 - i * 1900,
  })),
};
const OV_CONTEXT = {
  engineAvailable: true, activityCount: 386, conclusionCount: 142,
  lastDerivedAtMs: TL_NOW - 900000, coveredUntilMs: TL_NOW, backfilling: false,
  tokenUsage: { inputTokens: 0, outputTokens: 0, totalTokens: 0 },
  budgetTier: "balanced", subjectCount: 12, dismissedCount: 3, skippedWindows24h: 2,
};

// ── Recording-settings fixture (additive) ────────────────────────────────────
// Without this, get_recording_settings returned {} and every settings panel
// sat on "Loading settings…" — worse, a bindable prop then took undefined and
// Svelte threw props_invalid_value on EVERY re-render, which silently aborted
// the render and made the surface look frozen (⌘F filtering appeared broken
// when it was only unable to commit). Shipping-default-shaped, so a screenshot
// shows the app as a new user meets it.
const TL_MIC = { id: "BuiltInMicrophoneDevice", name: "MacBook Pro Microphone",
  isDefault: true, isAvailable: true, transport: "builtin" };
const TL_SETTINGS = {
  captureScreen: true, captureMicrophone: true, captureSystemAudio: true,
  segmentDurationSeconds: 300, screenFrameRate: 0.5,
  saveDirectory: "/Users/you/.mnema", autoStart: false,
  screenResolution: { mode: "preset", preset: "original" },
  videoBitrate: { mode: "preset", preset: "medium", customMbps: null },
  nativeCaptureDebugLoggingEnabled: false,
  pauseCaptureOnInactivity: true, idleTimeoutSeconds: 180,
  activityMode: "input", microphoneActivitySensitivity: 0.5,
  systemAudioActivitySensitivity: 0.5, microphoneVadAdapter: "webrtc",
  audioSpeechDetection: { detector: "webrtc" },
  metadata: { enabled: true, browserUrlMode: "off" },
  privacy: { excludedApps: [], filterSystemAudio: true },
  access: { askAiEnabled: true, askAiMaxToolCalls: 12, askAiWebFetchEnabled: false },
  aiRuntime: { enabled: false, providers: [], defaultModel: null, mcpServers: [
    // Two connectors so the MCP list draws its rows (hairline-separated, one
    // fill) instead of only its empty hint. One OAuth, one stdio.
    { id: "notion", label: "Notion", transport: "http", url: "https://mcp.notion.com/mcp",
      command: null, args: [], env: [], enabled: true, authMode: "oauth", enabledTools: null },
    { id: "filesystem", label: "Filesystem", transport: "stdio", url: null,
      command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "/Users/you"],
      env: [], enabled: false, authMode: "none", enabledTools: null },
  ] },
  userContext: { enabled: false, derivationBudgetTier: "balanced", backfillWindowDays: 7, backfillGoDeeper: false },
  semanticSearch: { enabled: SEM === "on", provider: "candle", modelId: SEM === "on" ? "nomic-embed-text-v1.5" : null },
  previewCacheTtlSeconds: 60, followTimelineLive: true,
  retentionPolicy: "never", appearance: "system",
  ocr: { enabled: true, provider: "apple_vision", modelId: null, language: null,
    recognitionMode: "accurate", languageCorrection: true,
    tesseractPageSegmentationMode: "auto", tesseractPreprocessMode: "grayscale",
    tesseractUpscaleFactor: 1, tesseractCharWhitelist: null },
  transcription: { enabled: true, microphoneEnabled: true, systemAudioEnabled: true,
    provider: "parakeet", modelId: null, language: "en", memoryMode: "balanced",
    idleUnloadSeconds: 120, chunkSeconds: 30 },
  speakerAnalysis: { separateSpeakers: true, recognizeSavedPeople: false,
    autoLabelOwner: false, provider: "speakrs", modelId: null, timeoutSeconds: 120 },
  developerOptionsEnabled: false,
};

// ── Transcription model fixture (additive) ───────────────────────────────────
// The model picker instrument draws one row per model of the selected provider
// with its download size priced against this Mac, so an empty provider list
// left it with nothing to render. Sizes are the real manifest figures; the
// provider set matches ADR 0047 (deepgram is a fourth provider, cloud).
const TX_MODEL = (provider, modelId, displayName, byteSize, management = "app_managed", status = "installed") => ({
  provider, modelId, displayName,
  description: displayName + " — fixture status for the render harness.",
  management, status, available: status === "installed" || status === "os_managed",
  availabilityStatus: null, installPath: byteSize ? "/Users/you/.mnema/models/" + modelId : null,
  missingFiles: [], failureMessage: null, licenseLabel: "Apache-2.0", sourceUrl: null,
  download: byteSize ? { byteSize, sha256: "0".repeat(64), url: "https://example.invalid" } : null,
});
const TX_MODELS = { modelsDirectory: "/Users/you/.mnema/models", providers: [
  { provider: "parakeet", displayName: "Parakeet", models: [
    TX_MODEL("parakeet", "parakeet-tdt-0.6b-v3-onnx-int8", "Parakeet v3 (int8)", 620000000),
    TX_MODEL("parakeet", "parakeet-tdt-0.6b-v3-onnx", "Parakeet v3", 2400000000, "app_managed", "missing"),
  ] },
  { provider: "local_whisper", displayName: "Local Whisper", models: [
    TX_MODEL("local_whisper", "base", "Whisper base", 148000000),
    TX_MODEL("local_whisper", "large-v3-turbo", "Whisper large-v3 turbo", 1600000000, "app_managed", "missing"),
    TX_MODEL("local_whisper", "large-v3", "Whisper large-v3", 6200000000, "app_managed", "missing"),
  ] },
  { provider: "apple_speech_on_device", displayName: "Apple Speech", models: [
    TX_MODEL("apple_speech_on_device", null, "Apple Speech (on-device)", 0, "os_managed", "os_managed"),
  ] },
  { provider: "deepgram", displayName: "Deepgram", models: [
    TX_MODEL("deepgram", "nova-3", "Deepgram nova-3", 0, "cloud", "missing"),
  ] },
] };

// ── Quick Access fixture (additive) ─────────────────────────────────────────
// Enough for search to return a drawable 3-up grid of REAL frames (the cells
// carry pictures, not grey rectangles), for the no-match state to have a query
// that genuinely matches nothing, and for the current-frame bar to have a shot
// with real pixel dimensions. Set QA=empty to shoot the no-match face.
const QA_MODE = ${JSON.stringify(process.env.QA ?? "")};
const QA_EMPTY = QA_MODE === "empty";
const QA_SHOT = (fill, ink, accent) => "data:image/svg+xml;base64," + btoa(
  '<svg xmlns="http://www.w3.org/2000/svg" width="698" height="392">' +
  '<rect width="698" height="392" fill="' + fill + '"/>' +
  '<rect width="698" height="34" fill="' + ink + '" opacity=".5"/>' +
  '<rect width="150" height="392" y="34" fill="' + ink + '" opacity=".28"/>' +
  Array.from({ length: 13 }, (_, i) =>
    '<rect x="180" y="' + (58 + i * 24) + '" width="' + (120 + ((i * 83) % 420)) +
    '" height="9" rx="2" fill="' + (i % 4 === 1 ? accent : ink) + '" opacity="' +
    (i % 4 === 1 ? ".9" : ".55") + '"/>').join("") +
  Array.from({ length: 9 }, (_, i) =>
    '<rect x="24" y="' + (58 + i * 26) + '" width="' + (60 + ((i * 47) % 80)) +
    '" height="8" rx="2" fill="' + ink + '" opacity=".4"/>').join("") +
  '</svg>');
const QA_APPS = [
  ["com.microsoft.VSCode", "Code", "webhook.rs — mnema", "#1b1b24", "#6fb8f0", "#8fd694"],
  ["com.apple.Safari", "Safari", "Polar webhook signatures — docs", "#f5f5f2", "#3d3d46", "#2c8ef3"],
  ["com.tinyspeck.slackmacgap", "Slack", "#mnema-dev — webhook thread", "#3b2544", "#e7e7e2", "#4ade80"],
  ["com.apple.Terminal", "Terminal", "cargo test webhook_verifier", "#0c0e0d", "#4ade80", "#f87171"],
  ["md.obsidian", "Obsidian", "Q3 launch — webhook checklist", "#f5f5f2", "#3d3d46", "#a259ff"],
  ["com.figma.Desktop", "Figma", "Checkout flow — webhook states", "#2c2c33", "#e7e7e2", "#f24e1e"],
];
const QA_SNIPPETS = [
  "the <mark>webhook</mark> verifier hashes the raw whsec_ string",
  "verifying a <mark>webhook</mark> signature requires the raw secret",
  "the <mark>webhook</mark> is still 400-ing on retries",
  "test webhook_verifier::<mark>webhook</mark>_raw_secret ... ok",
  "ship the <mark>webhook</mark> retry budget before Friday",
  "<mark>webhook</mark> failure state — inline error row",
];
const QA_FRAMES = QA_APPS.map(([appBundleId, appName, windowTitle, fill, ink, accent], i) => ({
  groupKey: "qa-frame-" + i,
  representativeFrame: {
    id: 5000 + i, sessionId: "sess-fixture", filePath: "/fixture/qa-" + i + ".png",
    capturedAt: new Date(TL_NOW - (i + 1) * 1800000).toISOString(),
    width: 2560, height: 1440, appBundleId, appName, windowTitle,
    url: null, ocrText: null, processorVersion: null, equivalenceHint: null,
    createdAt: new Date(TL_NOW).toISOString(), updatedAt: new Date(TL_NOW).toISOString(),
  },
  groupStartAt: new Date(TL_NOW - (i + 1) * 1800000 - 240000).toISOString(),
  groupEndAt: new Date(TL_NOW - (i + 1) * 1800000).toISOString(),
  matchCount: [3, 1, 5, 1, 2, 1][i], snippet: QA_SNIPPETS[i],
  appBundleId, appName, windowTitle,
  url: i === 1 ? "docs.polar.sh/webhooks" : null,
  thumbnailFrameId: 5000 + i, textSourceKind: "direct",
  hasSecretRedactions: i === 3, secretRedactionCount: i === 3 ? 1 : 0,
  foundByMeaning: i === 4,
}));
const QA_SHOTS = new Map(QA_APPS.map(([, , , fill, ink, accent], i) =>
  [5000 + i, QA_SHOT(fill, ink, accent)]));
const QA_AUDIO = [
  ["microphone", "the <mark>webhook</mark> is still 400-ing on the raw secret", 298],
  ["systemAudio", "we agreed the <mark>webhook</mark> retry budget stays at five", 154],
  ["microphone", "delivery ids in the <mark>webhook</mark> log line, please", 96],
].map(([sourceKind, snippet, seconds], i) => ({
  groupKey: "qa-audio-" + i,
  audioSegment: { ...TL_SEGS[0], id: 950 + i, sourceKind },
  sourceKind, spanStartMs: 0, spanEndMs: seconds * 1000,
  absoluteStartAt: new Date(TL_NOW - (i + 2) * 3600000).toISOString(),
  absoluteEndAt: new Date(TL_NOW - (i + 2) * 3600000 + seconds * 1000).toISOString(),
  matchCount: i + 1, snippet, alignedFrame: null,
  hasSecretRedactions: false, secretRedactionCount: 0, foundByMeaning: false,
}));
const QA_SEARCH = (query) => ({
  normalizedQuery: query, snapshotDocumentId: 1,
  frames: QA_EMPTY ? [] : QA_FRAMES, audio: QA_EMPTY ? [] : QA_AUDIO,
  hasMoreFrames: false, hasMoreAudio: false,
  appliedRefinements: {}, residualQuery: query, parseErrors: [],
});
// The current-frame shot: a real image, so the freshness readout can report a
// real pixel size instead of guessing one.
// QA=stale ages the grab past the staleness threshold (the warn pill + re-grab);
// QA=novision drops vision support (G2's upfront disclosure, before you type).
const QA_CURRENT_FRAME = () => ({
  imagePath: QA_SHOT("#2c2c33", "#e7e7e2", "#f24e1e"),
  capturedAtUnixMs: Date.now() - (QA_MODE === "stale" ? 46000 : 400),
  appName: "Figma", windowTitle: "Checkout flow — webhook states",
  excludedAppNames: ["1Password"], visionSupported: QA_MODE !== "novision",
  modelLabel: "gpt-oss-20b (local)",
});

// ── OCR / speaker / semantic model fixtures (additive) ───────────────────────
// These three commands used to fall through to the generic "anything matching
// /models|providers/ is an array" rule, which returns [] — the WRONG SHAPE.
// A status?.providers.find(...) / status?.models.find(...) chain then threw
// "Cannot read properties of undefined (reading 'find')" inside a $derived,
// which aborts the whole settings derivation chain: every panel silently fell
// back to its hardcoded defaults and no screenshot of Settings was true.
const OCR_MODELS = { modelsDirectory: "/Users/you/.mnema/models", providers: [
  { provider: "apple_vision", displayName: "Apple Vision", models: [{
    provider: "apple_vision", modelId: null, displayName: "Apple Vision (system)",
    description: "macOS-managed text recognition — no download.",
    management: "os_managed", status: "os_managed", available: true,
    installPath: null, missingFiles: [], failureMessage: null,
    licenseLabel: null, sourceUrl: null, download: null, runtimeMessage: null,
  }] },
  { provider: "tesseract", displayName: "Tesseract", models: [{
    provider: "tesseract", modelId: "tesseract-5.5.2", displayName: "Tesseract 5.5.2",
    description: "Bundled OCR engine for non-Apple-Vision workflows.",
    management: "app_managed", status: "missing", available: false,
    installPath: null, missingFiles: ["tesseract"], failureMessage: null,
    licenseLabel: "Apache-2.0", sourceUrl: null,
    download: { byteSize: 90000000, sha256: "0".repeat(64), url: "https://example.invalid" },
    runtimeMessage: null,
  }] },
] };
const SPK_MODELS = { modelsDirectory: "/Users/you/.mnema/models", providers: [
  { provider: "speakrs", displayName: "speakrs", models: [{
    provider: "speakrs", modelId: null,
    displayName: "speakrs · pyannote-community-1 + WeSpeaker",
    description: "On-device diarization and speaker embeddings on CoreML.",
    status: "installed", available: true,
    installPath: "/Users/you/.mnema/models/speakrs", missingFiles: [],
    failureMessage: null, licenseLabel: "MIT", sourceUrl: null,
    // 419 MB — the corrected registry figure (G8), not the old 31 MB.
    download: { byteSize: 419000000, sha256: "0".repeat(64), url: "https://example.invalid" },
  }] },
] };
const SEM_MODELS = { modelsDirectory: "/Users/you/.mnema/models", models: [{
  provider: "candle", modelId: "nomic-embed-text-v1.5", displayName: "Nomic Embed v1.5",
  description: "English embedding model, 768 dimensions.", tier: "english",
  dimension: 768, maxTokens: 2048, modelCode: "nomic-ai/nomic-embed-text-v1.5",
  approxDownloadBytes: 274000000, licenseLabel: "Apache-2.0",
  status: SEM === "on" ? "installed" : "missing", available: SEM === "on",
  installPath: "/Users/you/.mnema/models/nomic", missingFiles: [],
}] };

// ── User-context fixture (additive) ─────────────────────────────────────────
// The Journal / Subjects / Context destinations (pages 08–10). Activities cover
// today with the mockup's late-morning gap; conclusions span all four
// conviction tiers; trajectories carry REAL snapshotAtMs values across six
// weeks so the trace's time x-axis is provable in a screenshot. EMPTY=1 keeps
// every read empty, exactly as day one does.
const UC_AT = (h, m) => { const d = new Date(TL_NOW); d.setHours(h, m, 0, 0); return d.getTime(); };
const UC_ACTIVITIES = [
  ["Reading Polar's webhook signature docs", "research", "deep", 9, 14, 38,
   "Worked through how Polar signs its webhooks and confirmed the HMAC is keyed on the raw whsec_ string rather than the decoded key."],
  ["Design review with the product team", "meetings", "mixed", 10, 12, 41,
   "Walked the settings redesign end to end; the toolbar-tab shape stuck and the retention ladder was the only control anyone argued about."],
  ["Filing the sprint board", "organizing", null, 10, 56, 6,
   "Moved the finished round-4 cards and split the webhook fix into two."],
  ["Rewriting the licensing CRL cache", "creating", "deep", 11, 48, 74,
   "Replaced the eager fetch with a cached read and a staleness window, then rewrote the test to stop reaching for the network."],
  ["Launch sync — the webhook is still 400-ing", "meetings", "deep", 13, 2, 38,
   "Five people, one unresolved failure: the signature check passes locally and fails in production. Tom asked for delivery ids in the log before the next attempt."],
  ["answer_view.rs — parsing answers into typed blocks", "creating", "deep", 13, 48, 46,
   "Moved fence parsing off the frontend: the backend now hands over one render-ready view per turn instead of a string the UI has to guess at."],
  ["Slack triage", "communication", "distracted", 14, 36, 26,
   "Cleared the launch channel and answered the two webhook threads that could not wait."],
].map(([title, category, focus, h, m, mins, summary], i) => ({
  id: 80 + i, title, summary, category, focus,
  startedAtMs: UC_AT(h, m), endedAtMs: UC_AT(h, m) + mins * 60000,
  createdAtMs: UC_AT(h, m) + mins * 60000,
  evidence: [
    { subjectType: "frame", subjectId: 4000 - i * 3, capturedAtMs: UC_AT(h, m) + 300000, isHeadline: true },
    { subjectType: "frame", subjectId: 4001 - i * 3, capturedAtMs: UC_AT(h, m) + 900000, isHeadline: false },
  ],
}));
const UC_DAYS = 86400000;
const UC_CONCLUSION = (id, subject, statement, confidence, opts = {}) => ({
  id, subject, statement, confidence,
  status: opts.status ?? "visible", pinned: opts.pinned ?? false,
  formedAtMs: TL_NOW - (opts.ageDays ?? 21) * UC_DAYS,
  lastSupportedAtMs: TL_NOW - (opts.lastDays ?? 0.1) * UC_DAYS,
  updatedAtMs: TL_NOW - (opts.lastDays ?? 0.1) * UC_DAYS,
  evidence: [
    { activityId: 84, stance: "support", activityTitle: "Launch sync — the webhook is still 400-ing", activityStartedAtMs: UC_AT(13, 2) },
    { activityId: 80, stance: "support", activityTitle: "Reading Polar's webhook signature docs", activityStartedAtMs: UC_AT(9, 14) },
  ],
  replacedStatement: opts.replaced ?? null,
  replacedAtMs: opts.replaced ? TL_NOW - 7 * UC_DAYS : null,
});
const UC_CONCLUSIONS = [
  UC_CONCLUSION(1, "Mnema licensing", "Ships a one-time purchase with a one-year update window, verified offline.", 0.86, { pinned: true, ageDays: 42, replaced: "Ships a monthly subscription with a free tier" }),
  UC_CONCLUSION(2, "Mnema licensing", "Keys the Polar webhook HMAC on the raw whsec_ string, not the decoded key.", 0.74, { ageDays: 12, lastDays: 0.08 }),
  UC_CONCLUSION(3, "Mnema licensing", "Treats the revocation list as a cache with a staleness window, not a live fetch.", 0.51, { ageDays: 9, lastDays: 2 }),
  UC_CONCLUSION(4, "Mnema licensing", "Considered a model where updates are free forever.", 0.12, { status: "faded", ageDays: 40, lastDays: 24 }),
  UC_CONCLUSION(5, "Mnema licensing", "Wants the license dialog to never block launch.", 0.44, { ageDays: 15, lastDays: 4 }),
  UC_CONCLUSION(6, "Capture pipeline", "Treats a display going away as transient liveness, never a privacy failure.", 0.79, { ageDays: 35, lastDays: 0.3 }),
  UC_CONCLUSION(7, "Capture pipeline", "Keeps system audio as its own capture family on Core Audio taps.", 0.66, { ageDays: 20, lastDays: 1 }),
  UC_CONCLUSION(8, "Capture pipeline", "Caps capture segments at five minutes.", 0.6, { ageDays: 30, lastDays: 3 }),
  UC_CONCLUSION(9, "Capture pipeline", "Prefers rebuilding the tap over splicing a generation into a live writer.", 0.45, { ageDays: 8, lastDays: 2 }),
  UC_CONCLUSION(10, "How you work", "Does the deep work before lunch and leaves the afternoon to meetings.", 0.71, { ageDays: 28, lastDays: 1 }),
  UC_CONCLUSION(11, "How you work", "Writes the test before trusting a network-touching fix.", 0.55, { ageDays: 18, lastDays: 5 }),
  UC_CONCLUSION(12, "How you work", "Keeps Slack closed until the afternoon.", 0.4, { ageDays: 10, lastDays: 6 }),
  UC_CONCLUSION(13, "Deepgram streaming", "Reading the websocket protocol against the batch endpoint; no decision yet.", 0.54, { ageDays: 5, lastDays: 0.2 }),
  UC_CONCLUSION(14, "Deepgram streaming", "Leans toward keeping cloud transcription a provider property.", 0.37, { ageDays: 4, lastDays: 1 }),
  UC_CONCLUSION(15, "Speaker diarization", "Prefers speakrs over sortformer once a room has five or more speakers.", 0.47, { ageDays: 25, lastDays: 3 }),
  UC_CONCLUSION(16, "Speaker diarization", "Benchmarks with DER before adopting any provider.", 0.42, { ageDays: 22, lastDays: 8 }),
  UC_CONCLUSION(17, "Speaker diarization", "Caps embedding windows near a minute.", 0.3, { ageDays: 14, lastDays: 9 }),
  UC_CONCLUSION(18, "Onboarding rework", "Wants exactly two hard gates and nothing else blocking the finish.", 0.24, { ageDays: 6, lastDays: 0.5 }),
  UC_CONCLUSION(19, "Onboarding rework", "Anchors the pitch on ~270 MB per captured day.", 0.2, { ageDays: 6, lastDays: 2 }),
  UC_CONCLUSION(20, "Sortformer evaluation", "Benchmarked it and moved on — the four-speaker cap was the end of it.", 0.11, { status: "faded", ageDays: 44, lastDays: 40 }),
  UC_CONCLUSION(21, "Sortformer evaluation", "Considered shipping it as a fallback provider.", 0.08, { status: "faded", ageDays: 44, lastDays: 42 }),
];
// Six weeks of snapshots per conclusion, newest last, on REAL timestamps —
// rising toward the current confidence, floored at 0.04.
const UC_HISTORY = (c) => {
  const weeks = 6, points = [];
  for (let k = 0; k <= weeks * 2; k++) {
    const at = TL_NOW - (weeks * 7 - k * 3.5) * UC_DAYS;
    if (at < c.formedAtMs) continue;
    const t = k / (weeks * 2);
    const start = Math.min(0.54, Math.max(0.2, c.confidence - 0.32));
    points.push({ confidence: Math.max(0.04, +(start + (c.confidence - start) * t).toFixed(3)), snapshotAtMs: Math.round(at) });
  }
  return points.length >= 2 ? points : [
    { confidence: Math.max(0.04, c.confidence - 0.1), snapshotAtMs: c.formedAtMs },
    { confidence: c.confidence, snapshotAtMs: TL_NOW - 7200000 },
  ];
};
const UC_SUBJECT = (subject) => {
  const cs = UC_CONCLUSIONS.filter((c) => c.subject.toLowerCase() === String(subject ?? "").toLowerCase());
  return {
    subject: cs[0]?.subject ?? String(subject ?? ""),
    conclusions: cs,
    trajectories: cs.map((c) => ({ conclusionId: c.id, statement: c.statement, history: UC_HISTORY(c) })),
  };
};
const UC_AUTHORED = [
  ["I run the repo with Bun, never pnpm — commands go through the workspace root.", "tooling", 0.1],
  ["I'm building Mnema solo — design, Rust and the web site are all me.", "role", 21],
  ["I care about the app staying quiet about my machine — no fans, no surprise battery.", "values", 4],
].map(([text, topic, days], i) => ({
  id: 30 + i, text, topic,
  createdAtMs: TL_NOW - days * UC_DAYS, updatedAtMs: TL_NOW - days * UC_DAYS,
}));
const UC_DISMISSED = [
  ["How you work", "Prefers to work late into the evening.", 14],
  ["Mnema licensing", "Is evaluating a move to a subscription price.", 21],
].map(([subject, statement, days]) => ({ subject, statement, dismissedAtMs: TL_NOW - days * UC_DAYS }));

window.__TAURI_INTERNALS__ = {
  invoke: async (cmd, args) => {
    const req = (args && (args.request ?? args.payload ?? args)) || {};
    // Capture every Tauri event listener so EVAL can drive a streaming surface
    // (\`__RD4_EMIT("ask_ai_update", payload)\`) without a backend.
    if (cmd === "plugin:event|listen") {
      (window.__RD4_LISTENERS[args.event] ??= []).push(args.handler);
      return window.__RD4_LISTENERS.__n++;
    }
    if (cmd === "take_pending_license_deep_link") return null;          // the one legal null
    if (cmd === "search_capture") return QA_SEARCH(req.query ?? "");
    if (cmd === "get_frame_scrub_previews")
      return { previews: (req.frameIds ?? []).map((frameId) => ({
        frameId,
        preview: QA_SHOTS.has(frameId)
          ? { mimeType: "image/svg+xml", filePath: QA_SHOTS.get(frameId),
              sourceKind: "original_frame", hasSecretRedactions: false, secretRedactionCount: 0 }
          : null,
        missingReason: QA_SHOTS.has(frameId) ? null : "not_indexed",
      })) };
    if (cmd === "capture_current_frame") return QA_CURRENT_FRAME();
    if (cmd === "quick_recall_set_collapsed") return null;
    if (cmd === "ask_ai_availability") return { available: true, reason: null };
    if (cmd === "ask_ai_start" || cmd === "ask_ai_followup" || cmd === "ask_ai_cancel") return null;
    if (cmd === "ask_ai_snapshot") return null;
    if (cmd === "get_semantic_search_model_status") return { installed: true, downloading: false };
    if (cmd === "get_moments") return OV_EMPTY ? [] : OV_MOMENTS;
    if (cmd === "get_conversations") return OV_EMPTY ? [] : OV_CONVERSATIONS;
    if (cmd === "list_conversations") return OV_EMPTY ? [] : OV_ASKS;
    if (cmd === "get_latest_user_context_digest") return OV_EMPTY ? null : OV_DIGEST;
    if (cmd === "get_user_context_status") return OV_EMPTY ? { ...OV_CONTEXT, activityCount: 0, conclusionCount: 0, subjectCount: 0 } : OV_CONTEXT;
    if (cmd === "get_system_facts") return OV_EMPTY ? { ...OV_FACTS, diskFreeBytes: null, measuredBytesPerDay: null, measuredDays: 0 } : { ...OV_FACTS, ...SEM_FACTS };
    if (cmd === "get_usage_charts") return OV_EMPTY ? { ...OV_USAGE, timePerApp: [] } : OV_USAGE;
    if (cmd === "list_frames") return req.beforeId ? [] : TL_FRAMES;
    if (cmd === "get_frame") return TL_FRAMES[0];
    if (cmd === "get_latest_frame_in_range") return TL_FRAMES[0];
    if (cmd === "get_frame_preview" || cmd === "get_scrub_preview")
      return { mimeType: "image/svg+xml", filePath: TL_SHOT, sourceKind: "original_frame", hasSecretRedactions: false, secretRedactionCount: 0 };
    if (cmd === "get_scrub_preview_availability") return [];
    if (cmd === "list_audio_segments") return TL_SEGS;
    if (cmd === "get_audio_segment") return TL_SEGS[0];
    if (cmd === "list_day_coverage") return OV_EMPTY ? [] : TL_COVERAGE;
    if (cmd === "list_frame_summaries_in_range") return OV_EMPTY ? [] : TL_SUMMARIES(req);
    if (cmd === "get_audio_segment_media") return { mimeType: "audio/wav", dataBase64: TL_WAV };
    if (cmd === "get_audio_segment_waveform_peaks")
      return Array.from({ length: req.bucketCount ?? 96 }, (_, i) =>
        0.12 + 0.78 * Math.abs(Math.sin(i / 3.7)) * (0.45 + 0.55 * Math.abs(Math.cos(i / 11))));
    if (cmd === "list_speaker_turns") return TL_TURNS;
    // Journal (page 08) reads the digest for the DAY IT IS SHOWING, not the
    // latest one, and its "re-read" writes a fresh one. Without these the {}
    // fallback lands on \`digest.narrative\` and the read block renders a
    // truthy-but-blank digest instead of the fixture's prose.
    if (cmd === "get_user_context_digest" || cmd === "regenerate_user_context_digest")
      return OV_EMPTY ? null : { ...OV_DIGEST, rangeStartMs: req.startMs ?? OV_DIGEST.rangeStartMs,
                                rangeEndMs: req.endMs ?? OV_DIGEST.rangeEndMs };
    // One completed OCR job per frame, so the rail's OCR cluster renders its
    // real face (engine label + rerun + region count) instead of "no OCR data".
    if (cmd === "list_processing_jobs" && req.subjectType === "frame")
      return [{ id: 61, subjectType: "frame", subjectId: req.subjectId, processor: "ocr",
        status: "completed", attempts: 1, payloadJson: JSON.stringify({ provider: "apple_vision" }),
        queuedAt: new Date(TL_NOW - 60000).toISOString(), startedAt: null, finishedAt: null,
        lastError: null, createdAt: new Date(TL_NOW).toISOString(), updatedAt: new Date(TL_NOW).toISOString() }];
    if (cmd === "get_processing_result")
      return { id: 61, jobId: 61, processorVersion: "apple-vision", structuredPayloadJson: JSON.stringify({
        provider: "apple_vision",
        observations: TL_OCR,
      }), createdAt: new Date(TL_NOW).toISOString() };
    if (cmd === "get_recording_settings") return TL_SETTINGS;
    if (cmd === "list_user_context_activities") {
      if (OV_EMPTY) return [];
      const all = [...UC_ACTIVITIES].sort((x, y) => y.startedAtMs - x.startedAtMs);
      if (typeof req.startMs === "number" || typeof req.endMs === "number")
        return all.filter((a) => a.endedAtMs >= (req.startMs ?? 0) && a.startedAtMs <= (req.endMs ?? Infinity));
      const offset = req.offset ?? 0;
      return all.slice(offset, offset + (req.limit ?? all.length));
    }
    if (cmd === "list_user_context_conclusions") return OV_EMPTY ? [] : UC_CONCLUSIONS;
    if (cmd === "get_user_context_subject") return UC_SUBJECT(req.subject ?? args?.subject);
    if (cmd === "list_user_context_authored") return OV_EMPTY ? [] : UC_AUTHORED;
    if (cmd === "user_context_list_dismissed") return OV_EMPTY ? [] : UC_DISMISSED;
    if (cmd === "user_context_add_authored")
      return { id: 99, text: req.text ?? "", topic: req.topic ?? null, createdAtMs: Date.now(), updatedAtMs: Date.now() };
    if (cmd === "user_context_update_authored")
      return { id: req.id ?? 99, text: req.text ?? "", topic: req.topic ?? null, createdAtMs: Date.now(), updatedAtMs: Date.now() };
    if (cmd === "user_context_delete_authored" || cmd === "user_context_dismiss_conclusion" ||
        cmd === "user_context_restore_dismissed" || cmd === "user_context_set_pinned") return {};
    if (cmd === "get_ai_runtime_status")
      return { enabled: true, configured: true, available: true, defaultModel: null, reason: null };
    if (cmd === "get_microphone_controller_state" || cmd === "update_microphone_controller")
      return { devices: [TL_MIC], preference: { mode: "default", deviceId: null },
               disconnectPolicy: "fallback_to_default", effectiveDevice: TL_MIC };
    if (cmd === "get_storage_location") return "/Users/you/.mnema";
    // About tab. Without this the {} fallback lands on \`.app.version\` and the
    // whole About panel throws, so the tab renders the PREVIOUS panel's DOM.
    if (cmd === "get_app_update_status" || cmd === "check_for_app_updates" ||
        cmd === "set_app_update_channel")
      return { app: { productName: "Mnema", version: "0.4.1", identifier: "day.mnema",
                      platform: "darwin", arch: "aarch64" },
               channel: "stable", state: "upToDate", update: null, progress: null,
               error: null, lastCheckedAtUnixMs: Date.now() - 3600000, recordingActive: false };
    // The later {licensed:…} stub is not the real wire shape (a flat tagged
    // union), so the License row rendered blank. This one wins by position.
    if (cmd === "get_license_status")
      return { kind: "licensed", updateThroughMs: Date.now() + 3.15e10, inWindow: true,
               email: "you@example.com", name: "", activation: { state: "activated" } };
    if (cmd === "get_license_devices") return { used: 2, cap: 3 };
    if (cmd === "delete_recent_capture")
      return { windowSeconds: req.windowSeconds ?? 300, startedAt: "", endedAt: "",
               deletedCaptureSegments: 3, deletedFrames: 412, deletedAudioSegments: 2,
               deletedProcessingJobs: 0, deletedProcessingResults: 0, deletedBackgroundJobs: 0,
               deletedFrameBatches: 0, deletedSearchDocuments: 0, pendingFileTombstones: 0,
               fileDeleteErrors: 0 };
    if (cmd === "get_third_party_notices")
      return { plainText: "Mnema bundles open-source components.", entries: [
        { component: "speakrs", kind: "diarization", displayName: "speakrs",
          license: "MIT", sourceUrl: "https://github.com/…" },
        { component: "openblas", kind: "linear-algebra", displayName: "OpenBLAS",
          license: "BSD-3-Clause", sourceUrl: "https://github.com/…" }] };
    if (cmd === "get_app_notifications") return [];
    if (cmd === "get_license_status") return { licensed: { kind: "purchased", updatesUntilMs: Date.now() + 3.15e10 } };
    if (cmd === "get_audio_transcription_model_status") return TX_MODELS;
    if (cmd === "get_ocr_model_status") return OCR_MODELS;
    if (cmd === "mcp_oauth_statuses") return [{ id: "notion", state: "authorized" }];
    if (cmd === "get_speaker_analysis_model_status") return SPK_MODELS;
    if (cmd === "get_semantic_search_model_status") return SEM_MODELS;
    if (cmd === "list_semantic_search_supported_models") return SEM_MODELS.models;
    if (cmd.startsWith("update_"))
      return { domain: req.domain ?? args?.domain ?? "capture_sources", settings: { ...TL_SETTINGS, ...req } };
    if (cmd.startsWith("list_")) return [];
    if (cmd.startsWith("has_pending")) return false;
    if (cmd.startsWith("search_")) return [];
    // Anything a caller iterates or spreads must be an array, not {}.
    if (/failures|_list$|models|providers|apps|entries|items|results|conversations|moments|coverage|charts|languages|devices|sources|bindings$/.test(cmd)) return [];
    return {};
  },
  transformCallback: (c) => c,
  convertFileSrc: (x) => x,
  // getCurrentWindow() reads this synchronously; without it every surface that
  // touches its own window throws inside an $effect and the effects AFTER it
  // never run (which is why search looked dead here before).
  metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
};
window.__RD4_LISTENERS = { __n: 1 };
// Remember the last generated id. Ask AI keys its whole stream on a UUID minted
// inside the component, so EVAL has no other way to address the live turn.
const __rd4_uuid = crypto.randomUUID.bind(crypto);
crypto.randomUUID = () => (window.__RD4_LAST_UUID = __rd4_uuid());
// Deliver a fake backend event to every listener registered for it. The one
// door EVAL uses to shoot a mid-stream surface without a running backend.
window.__RD4_EMIT = (event, payload) => {
  for (const h of window.__RD4_LISTENERS[event] ?? []) h({ event, id: 0, payload });
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: async () => {} };
`;

await mkdir(outDir, { recursive: true });
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 2 });
await page.addInitScript(initScript);

const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

await page.goto(`http://localhost:${port}${route}`, { waitUntil: "networkidle" });
// data-theme is set pre-navigation, but SvelteKit's theme init may overwrite it.
await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
await page.waitForTimeout(900);

// The stub's `update_*` echo is not a real settings object, so the autosave
// path raises "Couldn't save …" toasts that are artifacts of the harness, not
// of the UI under test — and they sit on top of the bottom-right of every
// screenshot. Hidden by default; set SHOW_TOASTS=1 when the toast IS the thing
// you are verifying.
if (process.env.SHOW_TOASTS !== "1") {
  await page.addStyleTag({ content: ".toast-stack { display: none !important; }" });
}

// Optional interaction, so an INTERACTIVE state can be screenshotted too — a
// filtered settings list, an armed shortcut recorder, an open menu. Without
// this every shot is the at-rest state, and half this direction's states are
// not at rest.
//   CLICK='.some-button' TYPE_INTO='input[…]' TYPE_TEXT='theme' bun scripts/rd4-shot.mjs …
if (process.env.CLICK) {
  await page.click(process.env.CLICK);
  await page.waitForTimeout(400);
}
if (process.env.TYPE_INTO) {
  await page.click(process.env.TYPE_INTO);
  await page.type(process.env.TYPE_INTO, process.env.TYPE_TEXT ?? "", { delay: 40 });
  await page.waitForTimeout(600);
}
if (process.env.SCROLL_BY) {
  await page.evaluate((y) => {
    const el = document.querySelector(".settings-scroll, .ti-pane, main") ?? document.scrollingElement;
    el?.scrollBy(0, Number(y));
  }, process.env.SCROLL_BY);
  await page.waitForTimeout(400);
}

// Opt-in interaction before the shot, for states that only exist after a click
// or a keypress (a menu open, a drawer open). CLICK is a selector, PRESS a key
// name; SUFFIX names the PNG so two states of one route don't collide.
if (process.env.CLICK) {
  for (const sel of process.env.CLICK.split("|")) {
    await page.locator(sel.trim()).first().click({ timeout: 5000 }).catch((e) => errors.push(`CLICK ${sel}: ${e.message}`));
    await page.waitForTimeout(500);
  }
  await page.waitForTimeout(600);
}
if (process.env.PRESS) {
  for (const key of process.env.PRESS.split("|")) {
    await page.keyboard.press(key.trim()).catch((e) => errors.push(`PRESS ${key}: ${e.message}`));
    await page.waitForTimeout(700);
  }
  await page.waitForTimeout(600);
}
// HOVER parks the pointer a quarter of the way into an element — the only way
// to photograph a hover-only state (the timeline's ghost playhead + its time
// bubble). The pointer stays put through the screenshot.
if (process.env.HOVER) {
  const box = await page.locator(process.env.HOVER).first().boundingBox().catch(() => null);
  if (box) await page.mouse.move(box.x + box.width * 0.25, box.y + box.height / 2);
  else errors.push(`HOVER ${process.env.HOVER}: no box`);
  await page.waitForTimeout(700);
}

// Settings is one long scrolling column, so most sections are below the fold.
// SCROLL is a selector scrolled to the top of its pane before the shot — the
// only way to photograph a section that is not the first one, and the only way
// to prove the sticky group header is real rather than asserted.
if (process.env.SCROLL) {
  await page
    .locator(process.env.SCROLL)
    .first()
    .evaluate((el) => el.scrollIntoView({ block: "start" }))
    .catch((e) => errors.push(`SCROLL ${process.env.SCROLL}: ${e.message}`));
  await page.waitForTimeout(500);
}

// Last resort for a state no click/keypress can reach — chiefly a MID-STREAM
// surface, which only exists while a backend is pushing events. EVAL is page JS
// run just before the shot; `window.__RD4_EMIT(event, payload)` delivers a fake
// backend event to the app's real listeners.
if (process.env.EVAL) {
  await page.evaluate(process.env.EVAL).catch((e) => errors.push(`EVAL: ${e.message}`));
  await page.waitForTimeout(700);
}

const name = `${route.replace(/\W+/g, "_") || "root"}${process.env.SUFFIX ? "-" + process.env.SUFFIX : ""}-${theme}-${size}.png`;
// REPORT=1 prints a small DOM census before the browser closes, so a check can
// assert on BEHAVIOUR (did ⌘F actually filter?) rather than on a class name.
const report = process.env.REPORT === "1"
  ? await page.evaluate(() => ({
      rows: document.querySelectorAll(".setting-row").length,
      miss: document.querySelectorAll(".setting-row--miss").length,
      crumbs: document.querySelectorAll(".setting-row__crumb").length,
      finding: document.querySelector(".settings-shell")?.classList.contains("is-finding") ?? null,
    }))
  : null;

const out = path.join(outDir, name);
await page.screenshot({ path: out, fullPage: false });
await browser.close();

console.log(out);
if (errors.length) console.log("PAGE ERRORS:\n" + errors.slice(0, 8).join("\n"));
if (report) console.log("REPORT " + JSON.stringify(report));
