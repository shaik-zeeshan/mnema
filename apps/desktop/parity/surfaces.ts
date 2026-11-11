// Surface -> Svelte route -> mockup page -> states, for the parity harness (docs/gpui/PLAN.md P0).
// Every surface in BUILD-PROMPT §3.6 and every window in §3.4. State names are the mockup's own
// picker values (read from each page's state <select>), so `shoot.ts <surface> <state>` lines up
// with `docs/mockups/redesign/<page>.html?state=<state>`.

export interface Surface {
  /** Svelte route the Tauri side loads (path, plus query when the route reads one).
   *  Absent = no Tauri counterpart (the GPUI control gallery): that panel reads "not applicable". */
  route?: string;
  /** Tauri window label the stub reports (`windows.rs` AppWindow labels). */
  label: string;
  /** Window size the Tauri side, the mockup and the GPUI golden all render at. */
  viewport: { width: number; height: number };
  /** Mockup file in docs/mockups/redesign/. */
  page: string;
  /** Element screenshotted on the mockup page (the window, not the fake desktop). */
  selector: string;
  /** Per-state override of `selector` (pages that show each state in its own element). */
  stateSelector?: Record<string, string>;
  /** Mockup page size when the window is drawn inside a bigger page (defaults to `viewport`). */
  mockViewport?: { width: number; height: number };
  /** Optional click on the Tauri side after load, for views the route can't address by URL. */
  click?: string;
  /** Per-state clicks in order (overrides `click`), e.g. a tab then a row. */
  stateClick?: Record<string, string[]>;
  /** Per-state key presses on the Tauri side after the clicks (Playwright key names). */
  stateKeys?: Record<string, string[]>;
  /** Per-state fixture name under parity/fixtures/<surface>/ (defaults to the state). */
  stateFixture?: Record<string, string>;
  /** Per-state override of `route` (a state that opens a record by URL, e.g. `/chat?c=<id>`). */
  stateRoute?: Record<string, string>;
  /** Per-state mockup `#hash` (pages that pick a section by anchor, e.g. settings.html). */
  stateHash?: Record<string, string>;
  /** state -> mockup query string (without theme). */
  states: Record<string, string>;
}

const MAIN = { width: 1280, height: 800 };
const WIN = ".mx-window:visible";
const MIC_BAR = ".timeline-rail__audio-bar--microphone";

/** `{ id: "<param>=<id>" }` for each id; "" means the mockup's default (no param). */
const q = (param: string, ids: string, extra = ""): Record<string, string> =>
  Object.fromEntries(ids.split(/\s+/).filter(Boolean).map((id) => [id, [`${param}=${id}`, extra].filter(Boolean).join("&")]));

const INSIGHTS_ANY = "loading engine-off engine-off-midday not-set-up needs-reconnect offline derivation-off new-user";
const insights = (tab: string, label: string, ids: string): Surface => ({
  route: "/insights",
  label: "main",
  viewport: MAIN,
  page: `insights.html#${tab}`,
  selector: WIN,
  click: tab === "overview" ? undefined : `.in-bar button:has-text("${label}")`,
  states: { default: "", ...q("state", `${INSIGHTS_ANY} ${ids}`) },
});

const tabClick = (label: string) => `.in-bar button:has-text("${label}")`;
/** Overview's period capsule D · W · M switch (not the popover's copy). */
const unitClick = (label: string) => `.mx-date > .mx-date__units button[aria-label="${label}"]`;
/** Insights parity states: `[state, mockup anchor, Tauri clicks, mockup query]` (mirrors
 *  `insights::fixture::STATES`). */
const INSIGHTS_SET: [string, string, string[], string][] = [
  ["overview", "overview", [], ""],
  ["overview-empty", "overview", [], "state=new-user"],
  ["journal", "journal", [tabClick("Journal")], ""],
  ["journal-empty", "journal", [tabClick("Journal")], "state=new-user"],
  ["subjects", "subjects", [tabClick("Subjects")], ""],
  ["subjects-empty", "subjects", [tabClick("Subjects")], "state=new-user"],
  ["context", "context", [tabClick("Context")], ""],
  ["context-empty", "context", [tabClick("Context")], "state=new-user"],
  ["subject", "subjects", [tabClick("Subjects"), ".in-map__row", ".sd-row"], "state=detail"],
  ["receipt", "journal", [tabClick("Journal"), 'button:has-text("Receipt")'], ""],
  ["period-day-open", "overview", [unitClick("Day"), ".mx-date__btn"], ""],
  ["period-week-open", "overview", [unitClick("Week"), ".mx-date__btn"], ""],
  ["period-month-open", "overview", [unitClick("Month"), ".mx-date__btn"], ""],
];

/** Onboarding parity states: `[state, fixture, Tauri clicks]` (mirrors `onboarding::fixture::STATES`).
 *  Permissions with screen + mic granted stops at system audio, then Accessibility (Zen is installed). */
const obBtn = (label: string) => `.ob-foot button:has-text("${label}")`;
const OB_TO_CAPTURE = ['button:has-text("Begin setup")', obBtn("Continue"), obBtn("Capture & Storage")];
const OB_TO_SETUP = [...OB_TO_CAPTURE, obBtn("Your settings"), obBtn("Start")];
/** Change settings › AI features, "I have a key". */
const OB_TO_AI = [
  ...OB_TO_CAPTURE,
  obBtn("Your settings"),
  obBtn("Change settings"),
  '.tabs button:has-text("AI features")',
  '.mx-choice__opt:has-text("I have a key")',
];
const OB_AI_KEY = [...OB_TO_AI, "select:.ob-select=anthropic", 'input[aria-label="API key"]'];
const OB_AI_TYPED = ["s", "k", "-", "a", "n", "t", "-", "1", "2", "3"];
const OB_SET: [string, string, string[]][] = [
  ["welcome", "default", []],
  ["permissions", "default", ['button:has-text("Begin setup")']],
  ["permissions-granted", "granted", ['button:has-text("Begin setup")']],
  ["permissions-denied", "denied", ['button:has-text("Begin setup")']],
  ["capture", "granted", OB_TO_CAPTURE],
  ["capture-blocked", "tiny", OB_TO_CAPTURE],
  ["settings", "granted", [...OB_TO_CAPTURE, obBtn("Your settings")]],
  ["change", "granted", [...OB_TO_CAPTURE, obBtn("Your settings"), obBtn("Change settings")]],
  ["setup", "granted", OB_TO_SETUP],
  ["setup-done", "installed", OB_TO_SETUP],
  ["voice", "installed", [...OB_TO_SETUP, obBtn("Continue")]],
  ["finale", "finale", [...OB_TO_SETUP, obBtn("Continue"), obBtn("Skip")]],
  // AI setup (S14b). The stub answers instantly, so "verifying" lands live on the Tauri side.
  ["ai-choose", "granted", OB_TO_AI],
  ["ai-key", "granted", OB_AI_KEY],
  ["ai-verifying", "granted", OB_AI_KEY],
  ["ai-ready", "granted", OB_AI_KEY],
  ["ai-invalid", "ai-invalid", OB_AI_KEY],
  ["ai-chatgpt", "granted", [...OB_TO_AI, "select:.ob-select=chatgpt", 'button:has-text("Sign in with ChatGPT")']],
];
/** Typed keys after the clicks (the key field verifies itself 650 ms after the last one). */
const OB_KEYS: Record<string, string[]> = {
  "ai-key": OB_AI_TYPED.slice(0, 5),
  "ai-verifying": OB_AI_TYPED,
  "ai-ready": OB_AI_TYPED,
  "ai-invalid": OB_AI_TYPED,
};

/** Settings parity states: `[state, ?tab=, mockup anchor]` (mirrors `settings::fixture::STATES`). */
const SETTINGS_SETS: [string, string, string][] = [
  ["general", "appearance", "s-appearance"],
  ["general-shortcuts", "shortcuts", "s-shortcuts"],
  ["capture", "capture", "s-capture"],
  ["capture-video", "video", "s-video"],
  ["capture-privacy", "privacy", "s-privacy"],
  ["intelligence", "intelligence", "s-providers"],
  ["intelligence-askai", "user-context", "s-usercontext"],
  ["intelligence-ocr", "ocr", "s-ocr"],
  ["intelligence-speakers", "speakers", "s-speakers"],
  ["data", "storage", "s-storage"],
  ["data-access", "access", "s-access"],
  ["about", "license", "s-license"],
];

/** Provider-row states over Intelligence: `[state, Tauri clicks, keys, mockup query]` (mirrors
 *  `settings::fixture::PROVIDER_STATES`). The device-code step needs a sign-in the stub can't
 *  answer, so its Tauri side stops at the fresh ChatGPT row. */
const pvManage = (label: string) => `.setting-row:has(.setting-row__label:text-is("${label}")) button:has-text("Manage")`;
const pvAddChatgpt = ['button:has-text("Add provider")', '[role="menuitem"]:has-text("ChatGPT")'];
const PROVIDER_SETS: [string, string[], string[], string][] = [
  ["provider-manage-local", [pvManage("Ollama")], [], ""],
  ["provider-manage-key", [pvManage("Anthropic"), "#ai-provider-key-anthropic"], ["s", "k", "a", "b"], "state=key-saving"],
  ["provider-chatgpt", pvAddChatgpt, [], "state=chatgpt-signin"],
  ["provider-chatgpt-code", pvAddChatgpt, [], "state=chatgpt-signin"],
];

/** Flow states outside Intelligence + its leftovers: `[state, ?tab=, Tauri clicks, keys, mockup query]`
 *  (mirrors the matching `settings::fixture::MODEL_STATES`). Each has its own
 *  `fixtures/settings/<state>.json` where it needs more than the default answers. */
const rowBtn = (label: string, button: string) =>
  `.setting-row:has(.setting-row__label:text-is("${label}")) button:has-text("${button}")`;
const REST_SETS: [string, string, string[], string[], string][] = [
  ["shortcuts-conflict", "shortcuts", ['button[aria-label="Set shortcut for Open settings"]'], ["Meta+d"], "state=shortcut-conflict"],
  ["capture-sysaudio-hint", "capture", [], [], "state=sysaudio-blocked"],
  [
    "video-custom-errors",
    "video",
    ['[aria-label="Screen resolution"] [role="radio"]:has-text("Custom")', '[aria-label="Bitrate"]', '[role="option"]:has-text("Custom")'],
    [],
    "state=bitrate-invalid",
  ],
  ["storage-after", "storage", ['button:has-text("Browse")', 'button:has-text("Run cleanup now")', 'button:has-text("Delete last 5 min")'], [], "state=storage-restart"],
  ["license-overcap", "license", [], [], ""],
  ["privacy-add-app", "privacy", ['button:has-text("Add app")'], [], ""],
  ["ai-status-error", "intelligence", [], [], ""],
  ["ocr-unused-deleted", "ocr", [rowBtn("Delete unused models", "Delete unused")], [], ""],
];

export const SURFACES: Record<string, Surface> = {
  // Shared controls (S6): the GPUI gallery bin vs the kit gallery's left (current-theme) panes.
  gallery: {
    label: "main",
    viewport: { width: 900, height: 900 },
    page: "index.html",
    selector: '.g-pane[data-specimen=""]:not([data-theme-invert])',
    mockViewport: { width: 1440, height: 900 },
    stateSelector: { feedback: '.g-pane[data-specimen="feedback"]:not([data-theme-invert])' },
    states: { components: "", feedback: "" },
  },
  // Titlebar + status bar around the default surface. Fixtures: parity/fixtures/shell/*.json.
  shell: {
    route: "/",
    label: "main",
    viewport: MAIN,
    page: "timeline.html",
    selector: WIN,
    states: { populated: "", empty: "state=first-run" },
  },
  // StatusPop (status-bar popover) states; the Tauri side shows the shell, the popover needs a click.
  status: {
    route: "/",
    label: "main",
    viewport: MAIN,
    page: "status.html",
    selector: WIN,
    states: q("state", "ok catching-up display-asleep sysaudio-rebuild low-disk-soon engine-offline not-recording dev-on"),
  },
  // Timeline `/` incl. AudioDrawer (drawer-*), TimelineJumper (jump-*) and the bell (notifs-*).
  // The drawer states open the mic bar of parity/fixtures/timeline/drawer*.json (S9).
  timeline: {
    route: "/",
    label: "main",
    viewport: MAIN,
    page: "timeline.html",
    selector: WIN,
    stateFixture: { repair: "drawer", "drawer-expanded": "drawer" },
    stateClick: {
      drawer: [MIC_BAR],
      "drawer-expanded": [MIC_BAR, '.audio-drawer button:has-text("expand")'],
      repair: [MIC_BAR, ".audio-drawer .who >> nth=1"],
      "drawer-not-transcribed": [MIC_BAR],
    },
    states: {
      // No mockup state of its own: the expanded reader is drawn over `drawer`.
      "drawer-expanded": "state=drawer",
      ...q(
      "state",
      `default loading first-run starting load-error stale refreshing not-recording permission permission-helper
       screen-off display-asleep landed loading-more end-of-history keys sb-off sb-permission readonly trial
       sb-starting sb-idle-source paused-idle paused-user low-disk stopped privacy-retry privacy-restart
       sysaudio-blocked sysaudio-reconnecting engine-setup engine-offline preview-decoding preview-failed
       frame-missing frame-deleted menu-not-ready toast-copied toast-copy-failed ocr-running ocr-empty ocr-reused
       ocr-missing ocr-error ocr-off ocr-unpositioned audio-loading audio-error audio-none drawer drawer-loading
       drawer-processing drawer-waiting-network drawer-model-missing drawer-not-transcribed
       drawer-transcription-off drawer-no-speech drawer-failed drawer-rerun-failed drawer-playback-error
       speakers-running speakers-failed speakers-none speakers-off recognition-off repair repair-saving
       repair-error repair-no-people jump jump-loading jump-empty-day jump-error jump-no-frame notifs
       notifs-warn notifs-empty notifs-error`,
      ),
    },
  },
  "insights-overview": insights(
    "overview",
    "Overview",
    "digest-writing rereading digest-failed too-few empty-range load-error nothing-changed",
  ),
  "insights-journal": insights(
    "journal",
    "Journal",
    "digest-writing too-few being-written nothing-captured load-error receipt-loading receipt-audio receipt-expired receipt-no-speech",
  ),
  "insights-subjects": insights(
    "subjects",
    "Subjects",
    "none-yet sparse no-match updated load-error detail detail-loading detail-empty detail-error",
  ),
  "insights-context": insights("context", "Context", "empty load-error adding add-error over-cap"),
  // The S12 set (fixtures/insights/{populated,empty}.json): every tab x {populated, empty},
  // a subject page with its evidence drawer, and a Journal card with its receipt open.
  insights: {
    route: "/insights",
    label: "main",
    viewport: MAIN,
    page: "insights.html",
    selector: WIN,
    stateFixture: Object.fromEntries(
      INSIGHTS_SET.map(([state]) => [state, state.endsWith("-empty") ? "empty" : "populated"]),
    ),
    stateClick: Object.fromEntries(INSIGHTS_SET.map(([state, , clicks]) => [state, clicks])),
    stateHash: Object.fromEntries(INSIGHTS_SET.map(([state, tab]) => [state, tab])),
    states: Object.fromEntries(INSIGHTS_SET.map(([state, , , mock]) => [state, mock])),
  },
  chat: {
    route: "/chat",
    label: "main",
    viewport: MAIN,
    page: "chat.html",
    selector: WIN,
    // The S11 parity set (fixtures/chat/<state>.json): history empty / listed, and a chat open
    // mid-stream (served by ask_ai_snapshot), finished, and failed.
    stateRoute: { "mid-stream": "/chat?c=c-stream", done: "/chat?c=c-done", error: "/chat?c=c-error" },
    states: {
      default: "",
      empty: "state=history-empty",
      list: "",
      "mid-stream": "state=writing",
      done: "state=thread",
      ...q(
        "state",
        `thread thinking tools writing reattach stopped interrupted regenerate copied source-gone error rate-limited
         context-overflow empty-answer needs-reconnect offline local-down not-set-up ai-off models-loading
         models-failed models-none scope history-loading history-empty history-error search-empty rename deleted
         convo-loading convo-error missing`,
      ),
    },
  },
  // The mockup opens on Capture; `?tab=capture` lands the route there too. The S13 set
  // (fixtures/settings/default.json for every state): each group's top, then the deeper
  // sections of the tall groups, reached by `?tab=` on the route and `#s-…` on the mockup.
  settings: {
    route: "/settings?tab=capture",
    label: "main",
    viewport: MAIN,
    page: "settings.html",
    selector: WIN,
    stateRoute: Object.fromEntries([
      ...SETTINGS_SETS.map(([state, tab]) => [state, `/settings?tab=${tab}`]),
      ...PROVIDER_SETS.map(([state]) => [state, "/settings?tab=intelligence"]),
      ...REST_SETS.map(([state, tab]) => [state, `/settings?tab=${tab}`]),
    ]),
    stateHash: Object.fromEntries([
      ...SETTINGS_SETS.map(([state, , anchor]) => [state, anchor]),
      ...PROVIDER_SETS.map(([state]) => [state, "s-providers"]),
    ]),
    stateClick: Object.fromEntries([
      ...PROVIDER_SETS.map(([state, clicks]) => [state, clicks]),
      ...REST_SETS.filter(([, , clicks]) => clicks.length).map(([state, , clicks]) => [state, clicks]),
    ]),
    stateKeys: Object.fromEntries([
      ...PROVIDER_SETS.filter(([, , keys]) => keys.length).map(([state, , keys]) => [state, keys]),
      ...REST_SETS.filter(([, , , keys]) => keys.length).map(([state, , , keys]) => [state, keys]),
    ]),
    states: {
      default: "",
      ...Object.fromEntries(SETTINGS_SETS.map(([state]) => [state, ""])),
      ...Object.fromEntries(PROVIDER_SETS.map(([state, , , query]) => [state, query])),
      ...Object.fromEntries(REST_SETS.map(([state, , , , query]) => [state, query])),
      ...q(
        "state",
        `save-failed validation shortcuts-loading shortcut-listening shortcut-conflict capture-loading
         sysaudio-blocked vad-off res-checking res-original-only bitrate-invalid mic-loading mic-failed mic-none
         excl-empty excl-recommended gecko-denied ai-not-ready no-providers key-saving key-rejected ollama-down
         chatgpt-signin chatgpt-reconnect chatgpt-unreachable mcp-empty mcp-authorizing mcp-reconnect mcp-failed
         askai-unavailable uc-backfill uc-paused ocr-downloading ocr-failed ocr-missing stt-downloading stt-failed
         stt-missing apple-denied deepgram-rejected deepgram-unavailable spk-downloading voice-none voice-recording
         voice-review voice-checking voice-rejected voice-saved sem-downloading sem-failed sem-reindex
         storage-resolving storage-failed storage-restart cleanup-running cleanup-done cleanup-failed cli-missing
         cli-path cli-installing cli-failed access-loading access-error access-empty lic-checking lic-trial-ready
         lic-trial lic-readonly lic-revoked lic-pending lic-lapsed lic-devicelimit lic-outofwindow
         lic-refresh-failed lic-key-invalid lic-activating lic-activated upd-checking upd-available
         upd-downloading upd-restart upd-failed upd-incompatible upd-outofwindow`,
      ),
    },
  },
  // NSPanel window; the mockup draws it over a fake desktop, so shoot the panel only.
  "quick-recall": {
    route: "/quick-recall",
    label: "quick-recall",
    viewport: { width: 800, height: 700 },
    page: "recall.html",
    selector: "#rc",
    mockViewport: MAIN,
    // The S10 set (fixtures/quick-recall/<state>.json): `take_quick_recall_query` types the
    // query; Quick Look and the pill popover take a click, Recent a ↓. The Tauri side can't
    // stream an answer (no `ask_ai_update` events in the stub), so its ask states stop at
    // "Thinking…"; the mockup carries the answer.
    stateClick: { quicklook: ['.rc-hero button:has-text("Quick Look")'], filter: [".rc-tok"] },
    stateKeys: { recent: ["ArrowDown"] },
    states: {
      idle: "",
      quicklook: "state=look",
      "ask-done": "state=ask",
      filter: "q=pricing%20page%20in%20Arc&pill=app",
      ...q(
        "state",
        `recent results first-run loading indexing semantic-off semantic-building error parse-error no-results
         audio-pending capture-off look look-missing gate ask-streaming ask ask-stopped ask-error ask-no-provider
         ask-off ask-reconnect ask-unreachable ask-offline closed`,
      ),
    },
  },
  // `?state=` presets, plus the plain steps (`?step=`, with `&perm` / `&folder` where the step reads them).
  // The S14 set (mirrors `onboarding::fixture::STATES`): the Svelte route always opens on Welcome,
  // so each step is reached by the same clicks a user makes, over parity/fixtures/onboarding/*.json.
  onboarding: {
    route: "/onboarding",
    label: "onboarding",
    viewport: { width: 1120, height: 800 },
    page: "onboarding.html",
    selector: "#ob-win",
    stateFixture: Object.fromEntries(
      OB_SET.map(([state, file]) => [state, file]),
    ),
    stateClick: Object.fromEntries(OB_SET.map(([state, , clicks]) => [state, clicks])),
    stateKeys: OB_KEYS,
    states: {
      ...q("step", "welcome permissions capture settings change setup voice finale"),
      // AI states the mockup doesn't draw borrow its nearest one.
      "ai-choose": "state=ai-held",
      "ai-key": "state=ai-verifying",
      "ai-chatgpt": "state=ai-held",
      "permissions-granted": "step=permissions&perm=sysaudio",
      "permissions-denied": "state=perm-denied",
      "capture-blocked": "step=capture&folder=tiny",
      "setup-done": "state=setup-done",
      "permissions-mic-denied": "step=permissions&perm=mic-denied",
      "permissions-sysaudio": "step=permissions&perm=sysaudio",
      "permissions-done": "step=permissions&perm=done",
      "capture-work": "step=capture&folder=work",
      "capture-archive": "step=capture&folder=archive",
      "capture-offline": "step=capture&folder=offline",
      "capture-tiny": "step=capture&folder=tiny",
      ...q(
        "state",
        `perm-ask perm-denied perm-prior perm-helper perm-relaunch perm-resumed perm-mic-denied perm-sys-listening
         perm-sys-blocked perm-done storage-ok storage-tight storage-readonly storage-offline storage-full settings
         settings-denied change-ai-unready change-undo ai-held ai-verifying ai-invalid ai-none ai-empty ai-ready
         setup setup-held setup-failed setup-offline setup-nospace setup-cancelled setup-done voice-waiting
         voice-off voice-ready voice-busy voice-quiet voice-rejected voice-multi voice-saved finale-starting
         finale-running finale-downloads finale-dl-failed finale-idle finale-audio-only finale-helper
         finale-failed`,
      ),
    },
  },
  // cli-access-request window; access.html switches on `?view=`, not `?state=`.
  access: {
    route: "/access/request",
    label: "cli-access-request",
    viewport: { width: 520, height: 600 },
    page: "access.html",
    mockViewport: MAIN,
    selector: WIN,
    // S15: fixtures/access/<state>.json; `granted` = the request + Allow (the receipt; the
    // mockup draws none, so its column shows the request).
    stateClick: { granted: ['button:has-text("Allow access")'] },
    states: {
      ...q(
        "view",
        "request unnamed widen submitting submit-failed expired blocked loading load-failed no-request manage manage-loading manage-error empty cli-missing",
      ),
      granted: "view=request",
    },
  },
  update: {
    route: "/update",
    label: "update",
    viewport: { width: 480, height: 520 },
    page: "update.html",
    selector: "#up-win",
    mockViewport: MAIN,
    states: q(
      "state",
      "available checking uptodate check-failed downloading download-failed installing install-failed ready incompatible expired",
    ),
  },
  // Floating helper; drawn inside timeline.html's permission-helper state.
  "permission-helper": {
    route: "/permission-helper",
    label: "permission-helper",
    viewport: { width: 460, height: 62 },
    page: "timeline.html",
    selector: "#tl-helper",
    mockViewport: MAIN,
    // `granted`: the fixture's screen permission is on; the mockup draws only the wait.
    states: { waiting: "state=permission-helper", granted: "state=permission-helper" },
  },
  // Developer window (gated on developerOptionsEnabled); `mini` is the 300x120 always-on-top mode.
  debug: {
    route: "/debug",
    label: "debug",
    viewport: { width: 520, height: 640 },
    page: "developer.html",
    selector: "#dv-win",
    mockViewport: MAIN,
    // Fixtures: debug/default.json (the issues scenario), overview-ok.json, not-recording.json.
    stateClick: Object.fromEntries(
      ["Pipeline", "Capture", "AI", "Logs"].map((t) => [t.toLowerCase(), [`.dv-tabs button:has-text("${t}")`]]),
    ),
    states: q("state", "overview-ok overview-issues pipeline processor job capture capture-asleep ai logs not-recording"),
  },
  "debug-mini": {
    route: "/debug",
    label: "debug",
    viewport: { width: 300, height: 120 },
    page: "developer.html",
    selector: "#dv-mini",
    mockViewport: MAIN,
    click: 'button[aria-label="Mini mode"]',
    states: { mini: "state=mini" },
  },
};

/** Mockup URL for a surface state and theme. */
export function mockupUrl(root: string, s: Surface, state: string, theme: string): string {
  const [file, pageAnchor] = s.page.split("#");
  const hash = s.stateHash?.[state] ?? pageAnchor;
  const query = [s.states[state], `theme=${theme}`].filter(Boolean).join("&");
  return `file://${root}/docs/mockups/redesign/${file}?${query}${hash ? `#${hash}` : ""}`;
}
