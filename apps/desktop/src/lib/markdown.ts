import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import go from "highlight.js/lib/languages/go";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import markdown from "highlight.js/lib/languages/markdown";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";
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

// Syntax highlighting uses the highlight.js *core* build with exactly 16 grammars
// registered individually, so the other ~190 grammars never ship in the bundle.
// `ini` covers TOML (highlight.js has no separate `toml` module; `ini` registers
// the `toml` alias) and `xml` covers HTML/XML.
const HIGHLIGHT_LANGUAGES: Array<[string, Parameters<typeof hljs.registerLanguage>[1]]> = [
  ["javascript", javascript],
  ["typescript", typescript],
  ["python", python],
  ["rust", rust],
  ["go", go],
  ["java", java],
  ["sql", sql],
  ["json", json],
  ["yaml", yaml],
  ["bash", bash],
  ["xml", xml],
  ["css", css],
  ["markdown", markdown],
  ["ini", ini],
  ["swift", swift],
  ["kotlin", kotlin],
];

for (const [name, module] of HIGHLIGHT_LANGUAGES) {
  hljs.registerLanguage(name, module);
}

// Number of language grammars registered with highlight.js. Exposed as a tiny
// helper so tests can assert the registration count without reaching into the
// shared hljs instance.
export function registeredLanguageCount(): number {
  return hljs.listLanguages().length;
}

// Build a MarkdownIt instance with all of our hardening rules applied. The only
// behavior that differs between instances is whether fenced code blocks are
// syntax-highlighted (`highlightEnabled`); everything else — `html: false`,
// image stripping, link allowlisting — is identical. We use two configured
// instances rather than mutating shared module state to switch modes.
function createRenderer(highlightEnabled: boolean): MarkdownIt {
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

  // GFM task-list items: `- [ ] item` / `- [x] item` render as checkbox rows
  // (the Triggers document view's "action items as a toggleable checklist" —
  // client-side toggling only, no persistence, so the input is NOT disabled).
  //
  // SECURITY: the only markup injected is the static `<input>` literal below,
  // via an `html_inline` token WE create. `html: false` means the parser never
  // produces html_inline tokens from the source, so no source-derived content
  // can ride this path — the item's own text still renders through the normal
  // escaped inline rules.
  md.core.ruler.after("inline", "task_list", (state) => {
    const tokens = state.tokens;
    for (let i = 2; i < tokens.length; i++) {
      const inline = tokens[i];
      if (
        inline.type !== "inline" ||
        inline.children === null ||
        inline.children.length === 0 ||
        tokens[i - 1].type !== "paragraph_open" ||
        tokens[i - 2].type !== "list_item_open"
      ) {
        continue;
      }
      const first = inline.children[0];
      if (first.type !== "text") continue;
      const marker = /^\[([ xX])\] /.exec(first.content);
      if (marker === null) continue;
      tokens[i - 2].attrJoin("class", "task-item");
      first.content = first.content.slice(marker[0].length);
      const checkbox = new state.Token("html_inline", "", 0);
      checkbox.content =
        marker[1] === " "
          ? '<input class="task-checkbox" type="checkbox">'
          : '<input class="task-checkbox" type="checkbox" checked>';
      inline.children.unshift(checkbox);
    }
  });

  // Override fenced code blocks to emit our "code chrome" (a header strip with a
  // language label + a copy button, then the highlighted/plain code). The class
  // names and data attributes below are a stable contract that the answer
  // component styles and services — do not rename them here without updating it.
  //
  // SECURITY: this fence rule is the ONLY place in this renderer that emits
  // trusted (un-`escapeHtml`'d) HTML, and it is safe for three independent
  // reasons that must ALL stay true:
  //   1. The code body is the only dynamic content, and it is ALWAYS escaped —
  //      either by highlight.js (`hljs.highlight(...).value` escapes the source
  //      it tokenizes; the resulting markup is only highlight.js's own static
  //      `<span class="hljs-...">` wrapping) or by `md.utils.escapeHtml` in the
  //      plain/unknown-language path. `html: false` from the source is therefore
  //      still fully in force — no source HTML reaches the output unescaped.
  //   2. The language LABEL is always run through `md.utils.escapeHtml`.
  //   3. The header/copy markup is static and owned by us; the copy button is
  //      inert markup (a `data-copy-code` hook) serviced by a delegated click
  //      handler in the component — it carries no source-derived attributes.
  // Nobody may extend this rule to emit any other source-derived content without
  // escaping it; doing so would defeat the `html: false` XSS defense above.
  md.renderer.rules.fence = (tokens, idx) => {
    const token = tokens[idx];
    const code = token.content;
    const info = token.info ? md.utils.unescapeAll(token.info).trim() : "";
    const requestedLang = info.split(/\s+/g)[0] ?? "";
    const knownLang =
      requestedLang && hljs.getLanguage(requestedLang) ? requestedLang : "";

    // Label: the known language name, else the literal fence text ("text" when
    // empty). Always escaped before it reaches the markup.
    const label = md.utils.escapeHtml(knownLang || requestedLang || "text");

    let codeHtml: string;
    if (highlightEnabled && knownLang) {
      // highlight.js escapes the source it tokenizes, so this value is safe.
      codeHtml = hljs.highlight(code, {
        language: knownLang,
        ignoreIllegals: true,
      }).value;
    } else {
      // Plain mode, or unknown/empty language: escaped plaintext, no token spans.
      codeHtml = md.utils.escapeHtml(code);
    }

    return (
      `<div class="answer-code" data-lang="${label}">` +
      `<div class="answer-code__header">` +
      `<span class="answer-code__lang">${label}</span>` +
      `<button class="answer-code__copy" type="button" data-copy-code aria-label="Copy code">Copy</button>` +
      `</div>` +
      `<pre class="answer-code__pre hljs"><code>${codeHtml}</code></pre>` +
      `</div>`
    );
  };

  return md;
}

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

// Two configured instances sharing identical hardening, differing only in
// whether fenced code blocks are syntax-highlighted. `mdPlain` is the cheap
// streaming-path default (no token spans); `mdHighlight` is the deferred,
// fully-highlighted pass. We never toggle a single shared instance's mode.
const mdPlain = createRenderer(false);
const mdHighlight = createRenderer(true);

// Render Markdown source to a sanitized HTML string. `highlight` defaults to
// false (plain mode — used on the streaming path: no token spans, cheap). When
// true, fenced code blocks are syntax-highlighted via highlight.js.
export function renderMarkdown(source: string, options?: { highlight?: boolean }): string {
  const md = options?.highlight ? mdHighlight : mdPlain;
  return md.render(source);
}
