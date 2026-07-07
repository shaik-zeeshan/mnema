import sitemap from "@astrojs/sitemap";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://mnema.day",
  output: "static",
  // /activate is a key-in-hash landing page reached only from the license email —
  // keep it out of the public sitemap so crawlers never advertise it.
  integrations: [sitemap({ filter: (page) => !page.includes("/activate") })],
});
