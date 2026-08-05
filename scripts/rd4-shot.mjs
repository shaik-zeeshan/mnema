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
window.__TAURI_INTERNALS__ = {
  invoke: async (cmd, args) => {
    const req = (args && (args.request ?? args.payload ?? args)) || {};
    if (cmd === "take_pending_license_deep_link") return null;          // the one legal null
    if (cmd === "get_app_notifications") return [];
    if (cmd === "get_license_status") return { licensed: { kind: "purchased", updatesUntilMs: Date.now() + 3.15e10 } };
    if (cmd === "get_audio_transcription_model_status") return { providers: [] };
    if (cmd.startsWith("update_")) return { ...req, domain: req.domain ?? args?.domain ?? "capture" };
    if (cmd.startsWith("list_")) return [];
    if (cmd.startsWith("has_pending")) return false;
    if (cmd.startsWith("search_")) return [];
    // Anything a caller iterates or spreads must be an array, not {}.
    if (/failures|_list$|models|providers|apps|entries|items|results|conversations|moments|coverage|charts|languages|devices|sources|bindings$/.test(cmd)) return [];
    return {};
  },
  transformCallback: (c) => c,
  convertFileSrc: (x) => x,
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

const name = `${route.replace(/\W+/g, "_") || "root"}-${theme}-${size}.png`;
const out = path.join(outDir, name);
await page.screenshot({ path: out, fullPage: false });
await browser.close();

console.log(out);
if (errors.length) console.log("PAGE ERRORS:\n" + errors.slice(0, 8).join("\n"));
