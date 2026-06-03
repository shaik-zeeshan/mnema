import MarkdownIt from "markdown-it";

// Renderer for Ask AI (PI) answers. The model streams Markdown — headings,
// lists, bold/italic, inline code, fenced code, blockquotes, links and tables —
// so we parse it to HTML rather than dumping the raw source.
//
// Security: this output is rendered with {@html ...} inside the Tauri webview,
// where an XSS would have access to the brokered capture surface. Ask AI text
// can transitively contain attacker-influenced content (e.g. OCR of a hostile
// page), so we never trust it as HTML:
//   - `html: false` escapes any raw HTML tags in the source instead of emitting
//     them, which is our primary XSS defense (no extra sanitizer needed).
//   - markdown-it's built-in `validateLink` already rejects javascript:,
//     vbscript:, file: and data: (except a few image types) URLs.
//   - we additionally strip image rendering and allowlist link schemes below,
//     because attacker-influenced source must not be able to trigger automatic
//     network fetches (tracking beacons via `![](http://evil/x.png)`) or launch
//     external custom-scheme handlers.
const md = new MarkdownIt({
  html: false, // never emit raw HTML from the source
  linkify: true, // turn bare URLs into links
  breaks: true, // treat single newlines as <br>, matching streamed prose
  typographer: false,
});

// Disable image rendering entirely. A Markdown image (`![alt](url)`) otherwise
// becomes an `<img src=url>` that the webview fetches automatically on render —
// an exfiltration/tracking channel driven by untrusted model/capture text. We
// render the alt text (if any) as plain text instead, so nothing is fetched.
md.renderer.rules.image = (tokens, idx) => {
  const alt = tokens[idx].content;
  return alt ? md.utils.escapeHtml(alt) : "";
};

// Schemes a rendered link may carry. markdown-it's validateLink already blocks
// javascript:/vbscript:/file:/data:, but still permits arbitrary custom schemes
// (e.g. `someapp://...`) that the webview/opener could hand to an external
// handler. We allowlist only web + mail links; relative/anchor links (no scheme)
// are also fine. Anything else is rendered as inert text (see link_open).
const ALLOWED_LINK_SCHEMES = ["http:", "https:", "mailto:"];

function isAllowedLinkHref(href: string | null): boolean {
  if (href === null) {
    return false;
  }
  const trimmed = href.trim();
  if (trimmed === "") {
    return false;
  }
  // Same-document / relative links (anchor, query, absolute path) carry no
  // scheme and navigate nowhere external, so they're inert and allowed. A
  // protocol-relative `//host` href DOES reach the network, so it's excluded
  // here and falls through to the scheme check (which rejects it).
  if (/^[#?/]/.test(trimmed) && !trimmed.startsWith("//")) {
    return true;
  }
  // Anything else must declare an allowlisted scheme. No scheme (e.g. a bare
  // `//host` protocol-relative href) is rejected.
  const schemeMatch = /^([a-z][a-z0-9+.-]*):/i.exec(trimmed);
  if (schemeMatch === null) {
    return false;
  }
  return ALLOWED_LINK_SCHEMES.includes(`${schemeMatch[1].toLowerCase()}:`);
}

// Open links in the user's browser via the Tauri opener rather than navigating
// the webview. We can't call the opener from inside the renderer, so we mark
// every link with rel/target and a data attribute; the component intercepts
// clicks on these and routes them through `@tauri-apps/plugin-opener`.
const defaultLinkOpen =
  md.renderer.rules.link_open ??
  ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));

md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
  const token = tokens[idx];
  // Only http/https/mailto links are openable. A disallowed scheme is stripped
  // to an inert anchor (no href, not tagged for the opener) so it neither
  // navigates the webview nor reaches a custom external handler.
  if (!isAllowedLinkHref(token.attrGet("href"))) {
    const hrefIndex = token.attrIndex("href");
    if (hrefIndex >= 0 && token.attrs) {
      token.attrs.splice(hrefIndex, 1);
    }
    return defaultLinkOpen(tokens, idx, options, env, self);
  }
  token.attrSet("data-external", "true");
  token.attrSet("rel", "noopener noreferrer");
  token.attrSet("target", "_blank");
  return defaultLinkOpen(tokens, idx, options, env, self);
};

// Render Markdown source to a sanitized HTML string.
export function renderMarkdown(source: string): string {
  return md.render(source);
}
