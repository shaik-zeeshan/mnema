import sitemap from "@astrojs/sitemap";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://mnema.day",
  output: "static",
  integrations: [sitemap()],
  // Cloudflare quick tunnels serve the dev server under a random *.trycloudflare.com
  // host; Vite's DNS-rebinding guard blocks anything but localhost by default.
  vite: { server: { allowedHosts: [".trycloudflare.com"] } },
});
