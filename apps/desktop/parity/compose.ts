// bun parity/compose.ts <dir>
// Lays <dir>/{gpui,tauri,mockup}.png side by side, labelled, into <dir>/composite.png.
// Uses Playwright (already here for shoot.ts) because ImageMagick isn't installed on dev Macs.
// A missing panel becomes a labelled grey box the size of the Tauri shot (e.g. "GPUI: not built").
import { chromium } from "@playwright/test";
import { existsSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2];
if (!dir || !existsSync(join(dir, "tauri.png"))) {
  console.error("usage: bun parity/compose.ts <dir containing tauri.png>");
  process.exit(2);
}

const panel = (file: string, label: string) =>
  existsSync(join(dir, file))
    ? `<figure><figcaption>${label}</figcaption><img src="${file}"></figure>`
    : `<figure><figcaption>${label}</figcaption><div class="missing">${label}: not built</div></figure>`;

const html = `<!doctype html><meta charset="utf-8"><style>
  body { margin: 0; padding: 16px; background: #1b1b1f; display: inline-flex; gap: 16px; align-items: flex-start;
         font: 600 14px -apple-system, system-ui, sans-serif; color: #e8e8ea; }
  figure { margin: 0; display: flex; flex-direction: column; gap: 8px; }
  img { display: block; outline: 1px solid #3a3a40; }
  .missing { display: grid; place-items: center; background: #5a5a60; color: #f2f2f4; font-size: 20px;
             width: var(--w); height: var(--h); outline: 1px solid #3a3a40; }
</style>${panel("gpui.png", "GPUI")}${panel("tauri.png", "Tauri")}${panel("mockup.png", "Mockup")}
<script>
  const t = new Image();
  t.onload = () => { for (const m of document.querySelectorAll(".missing")) { m.style.setProperty("--w", t.naturalWidth + "px"); m.style.setProperty("--h", t.naturalHeight + "px"); } document.body.dataset.ready = "1"; };
  t.src = "tauri.png";
</script>`;

const page = join(dir, "composite.html");
writeFileSync(page, html);
const browser = await chromium.launch();
try {
  const p = await browser.newPage();
  await p.goto(`file://${page}`);
  await p.waitForSelector("body[data-ready]");
  await p.evaluate(() => Promise.all([...document.images].map((i) => i.decode())));
  await p.locator("body").screenshot({ path: join(dir, "composite.png") });
} finally {
  await browser.close();
  rmSync(page);
}
console.log(join(dir, "composite.png"));
