import sitemap from "@astrojs/sitemap";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://mnema.day",
  output: "static",
  // /license/open is a noindex checkout bounce page — keep it out of the sitemap.
  integrations: [sitemap({ filter: (page) => !page.includes("/license/open") })],
  // Cloudflare quick tunnels serve the dev server under a random *.trycloudflare.com
  // host; Vite's DNS-rebinding guard blocks anything but localhost by default.
  vite: { server: { allowedHosts: [".trycloudflare.com"] } },
});
