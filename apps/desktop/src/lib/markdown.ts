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
const md = new MarkdownIt({
  html: false, // never emit raw HTML from the source
  linkify: true, // turn bare URLs into links
  breaks: true, // treat single newlines as <br>, matching streamed prose
  typographer: false,
});

// Open links in the user's browser via the Tauri opener rather than navigating
// the webview. We can't call the opener from inside the renderer, so we mark
// every link with rel/target and a data attribute; the component intercepts
// clicks on these and routes them through `@tauri-apps/plugin-opener`.
const defaultLinkOpen =
  md.renderer.rules.link_open ??
  ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));

md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
  const token = tokens[idx];
  token.attrSet("data-external", "true");
  token.attrSet("rel", "noopener noreferrer");
  token.attrSet("target", "_blank");
  return defaultLinkOpen(tokens, idx, options, env, self);
};

// Render Markdown source to a sanitized HTML string.
export function renderMarkdown(source: string): string {
  return md.render(source);
}
