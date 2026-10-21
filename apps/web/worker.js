// First-party Umami proxy: blockers list cloud.umami.is, not mnema.day/_e/*.
// Only these two routes are proxied, so this can't be used as an open proxy.
const ROUTES = {
  "GET /_e/s.js": "https://cloud.umami.is/script.js",
  "POST /_e/api/send": "https://gateway.umami.is/api/send",
};

export default {
  async fetch(request, env) {
    const { pathname } = new URL(request.url);
    const upstream = ROUTES[`${request.method} ${pathname}`];
    if (!upstream) return env.ASSETS.fetch(request);

    const headers = new Headers();
    for (const name of ["content-type", "user-agent"]) {
      const value = request.headers.get(name);
      if (value) headers.set(name, value);
    }
    // Umami derives geo from the visitor's IP; without this it sees the Worker's.
    const ip = request.headers.get("cf-connecting-ip");
    if (ip) headers.set("x-forwarded-for", ip);

    return fetch(upstream, { method: request.method, headers, body: request.body });
  },
};
