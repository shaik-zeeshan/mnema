// bun parity/shoot.ts <surface> <state> <theme>
// Screenshots the Svelte route (Tauri IPC stubbed) and the mockup for one surface x state x theme into
// target/parity/<surface>/<state>-<theme>/{tauri,mockup}.png, plus tauri-calls.json (what the page invoked).
import { chromium, type Browser, type BrowserContext } from "@playwright/test";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, openSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { SURFACES, mockupUrl, type Surface } from "./surfaces";
import { installTauriStub, invokedCommands, type CommandMap } from "./tauri-stub";

// The fixture's "now": every fixture timestamp is relative to it.
export const NOW = new Date("2026-11-04T14:32:00+05:30");
const TIMEZONE = "Asia/Kolkata";
const VITE = "http://localhost:1420";
const ROOT = resolve(import.meta.dir, "../../..");

async function viteUp(): Promise<boolean> {
  try {
    return (await fetch(VITE)).ok;
  } catch {
    return false;
  }
}

/** Reuse a Vite already on :1420, or start one detached (left running for the next shot). */
async function ensureVite(): Promise<void> {
  if (await viteUp()) return;
  const log = join(ROOT, "target/parity/vite.log");
  mkdirSync(join(ROOT, "target/parity"), { recursive: true });
  const fd = openSync(log, "a");
  spawn("bun", ["run", "dev"], { cwd: join(ROOT, "apps/desktop"), detached: true, stdio: ["ignore", fd, fd] }).unref();
  for (let i = 0; i < 120; i++) {
    if (await viteUp()) return;
    await Bun.sleep(500);
  }
  throw new Error(`Vite didn't answer on :1420 within 60 s; see ${log}`);
}

/** parity/fixtures/<surface>/<state>.json, else <surface>/default.json (gates such as /debug's developer
 *  options), else {}: every unmocked command gets the stub's empty-of-its-type default.
 *  `$REPO` in a fixture is the repo root, so file paths (frame images) resolve on any checkout. */
function fixture(surface: string, state: string): CommandMap {
  for (const name of [state, "default"]) {
    const path = join(import.meta.dir, "fixtures", surface, `${name}.json`);
    if (existsSync(path)) return JSON.parse(readFileSync(path, "utf8").replaceAll("$REPO", ROOT));
  }
  return {};
}

async function shoot(browser: Browser, surfaceId: string, state: string, theme: "dark" | "light"): Promise<void> {
  const s = SURFACES[surfaceId];
  const out = join(ROOT, "target/parity", surfaceId, `${state}-${theme}`);
  mkdirSync(out, { recursive: true });
  const context = await browser.newContext({ viewport: s.viewport, colorScheme: theme, timezoneId: TIMEZONE });

  // Tauri side: the real Svelte route on stubbed IPC (skipped for surfaces with no route).
  if (s.route) await shootTauri(context, s, surfaceId, state, out);

  // Mockup side: the window element only.
  const mock = await context.newPage();
  if (s.mockViewport) await mock.setViewportSize(s.mockViewport);
  await mock.clock.setFixedTime(NOW);
  await mock.goto(mockupUrl(ROOT, s, state, theme), { waitUntil: "load" });
  await mock.evaluate(() => document.fonts.ready);
  await mock.waitForTimeout(500);
  const selector = s.stateSelector?.[state] ?? s.selector;
  await mock.locator(selector).first().screenshot({ path: join(out, "mockup.png"), animations: "disabled" });

  await context.close();
  console.log(out);
}

async function shootTauri(
  context: BrowserContext,
  s: Surface,
  surfaceId: string,
  state: string,
  out: string,
): Promise<void> {
  const tauri = await context.newPage();
  const errors: string[] = [];
  tauri.on("pageerror", (e) => errors.push(e.message));
  await tauri.clock.setFixedTime(NOW);
  await installTauriStub(tauri, { label: s.label, map: fixture(surfaceId, state) });
  await tauri.goto(`${VITE}${s.stateRoute?.[state] ?? s.route}`, { waitUntil: "networkidle" });
  if (s.click) await tauri.locator(s.click).first().click();
  await tauri.evaluate(() => document.fonts.ready);
  await tauri.waitForTimeout(800); // let $effects settle after the stubbed invokes resolve
  await tauri.screenshot({ path: join(out, "tauri.png"), animations: "disabled" });
  writeFileSync(join(out, "tauri-calls.json"), JSON.stringify(await invokedCommands(tauri), null, 2));
  for (const e of errors) console.error(`[tauri page error] ${e}`);
}

// State defaults to the surface's first; theme defaults to both. Prints one output dir per line.
if (import.meta.main) {
  const [surfaceId, stateArg, themeArg] = process.argv.slice(2);
  const s = SURFACES[surfaceId];
  const state = stateArg || (s && Object.keys(s.states)[0]);
  const themes = themeArg ? [themeArg] : ["dark", "light"];
  if (!s || !(state in s.states) || themes.some((t) => t !== "dark" && t !== "light")) {
    console.error("usage: bun parity/shoot.ts <surface> [state] [dark|light]");
    console.error(s ? `states: ${Object.keys(s.states).join(" ")}` : `surfaces: ${Object.keys(SURFACES).join(" ")}`);
    process.exit(2);
  }
  if (s.route) await ensureVite();
  const browser = await chromium.launch();
  try {
    for (const theme of themes) await shoot(browser, surfaceId, state, theme as "dark" | "light");
  } finally {
    await browser.close();
  }
}
