import type { APIRoute } from "astro";

export const GET: APIRoute = ({ site }) =>
  new Response(
    `User-agent: *\nAllow: /\nDisallow: /activate\n\nSitemap: ${new URL("sitemap-index.xml", site)}\n`,
  );
