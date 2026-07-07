import sitemap from "@astrojs/sitemap";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://mnema.day",
  output: "static",
  integrations: [sitemap()],
});
