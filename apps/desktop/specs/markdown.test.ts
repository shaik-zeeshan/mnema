import { describe, expect, test } from "bun:test";
import { isOpenableHref, registeredLanguageCount, renderMarkdown } from "../src/lib/markdown";
import hljs from "highlight.js/lib/core";

describe("renderMarkdown code highlighting", () => {
  test("highlight mode emits highlight.js token spans for a known language", () => {
    const out = renderMarkdown("```js\nconst x = 1;\n```", { highlight: true });
    expect(out).toContain('class="hljs-');
    expect(out).toContain('class="answer-code"');
  });

  test("plain mode emits no token spans (escaped plaintext in <code>)", () => {
    const source = "```js\nconst x = 1;\n```";
    expect(renderMarkdown(source, { highlight: false })).not.toContain("hljs-");
    // default is plain mode
    expect(renderMarkdown(source)).not.toContain("hljs-");
  });

  test("unknown/empty language fence renders escaped plaintext with label 'text'", () => {
    const out = renderMarkdown("```\nplain code\n```", { highlight: true });
    expect(out).toContain('<span class="answer-code__lang">text</span>');
    expect(out).toContain('data-lang="text"');
    expect(out).not.toContain("hljs-");
    expect(out).toContain("plain code");
  });
});

describe("renderMarkdown code-block chrome", () => {
  test("emits the answer-code wrapper, copy button, and Copy text", () => {
    const out = renderMarkdown("```js\nconst x = 1;\n```", { highlight: true });
    expect(out).toContain('class="answer-code"');
    expect(out).toContain('class="answer-code__copy"');
    expect(out).toContain("data-copy-code");
    expect(out).toContain(">Copy</button>");
    expect(out).toContain('class="answer-code__pre hljs"');
  });
});

describe("renderMarkdown XSS hardening", () => {
  test("html: false holds — raw HTML in code content is escaped (highlight mode)", () => {
    const source = '```js\nconst x = "<script>alert(1)</script>";\n```';
    const out = renderMarkdown(source, { highlight: true });
    expect(out).not.toContain("<script>");
    expect(out).toContain("&lt;script&gt;");
  });

  test("html: false holds — raw HTML in code content is escaped (plain mode)", () => {
    const source = '```js\nconst x = "<script>alert(1)</script>";\n```';
    const out = renderMarkdown(source, { highlight: false });
    expect(out).not.toContain("<script>");
    expect(out).toContain("&lt;script&gt;");
  });

  test("raw HTML in prose is escaped, not emitted", () => {
    const out = renderMarkdown('hello <img src=x onerror="alert(1)"> world');
    expect(out).not.toContain("<img");
    expect(out).toContain("&lt;img");
  });

  test("image markdown renders no <img>, just alt text", () => {
    const out = renderMarkdown("![alt text](http://evil/x.png)");
    expect(out).not.toContain("<img");
    expect(out).toContain("alt text");
  });

  test("https link gets data-external + target=_blank", () => {
    const out = renderMarkdown("[link](https://example.com)");
    expect(out).toContain('data-external="true"');
    expect(out).toContain('target="_blank"');
    expect(out).toContain('href="https://example.com"');
  });

  test("javascript: link is rendered inert (no anchor, no href, no data-external)", () => {
    // markdown-it's validateLink rejects javascript:, so it never becomes an
    // anchor at all — the source stays literal text. Either way the property we
    // care about holds: nothing navigable/openable is emitted.
    const out = renderMarkdown("[bad](javascript:alert(1))");
    expect(out).not.toContain("data-external");
    expect(out).not.toContain("href=");
    expect(out).not.toContain("<a");
  });
});

describe("highlight.js registration", () => {
  test("exactly 16 languages are registered", () => {
    expect(registeredLanguageCount()).toBe(16);
    expect(hljs.listLanguages().length).toBe(16);
  });

  test("the toml alias resolves to ini", () => {
    expect(hljs.getLanguage("toml")).toBeTruthy();
  });
});

describe("isOpenableHref", () => {
  // The open-time gate, deliberately stricter than the render-time allowlist:
  // every rendered href that reaches `openUrl` (Ask AI answers, the update
  // window's release notes) is attacker-influenceable — model output over OCR'd
  // content, and feed notes injected into latest.json after signing.
  test("opens real web and mail links", () => {
    expect(isOpenableHref("https://github.com/shaik-zeeshan/mnema/compare/v1...v2")).toBe(true);
    expect(isOpenableHref("http://example.com")).toBe(true);
    expect(isOpenableHref("mailto:hi@mnema.day")).toBe(true);
  });

  test("refuses scheme-less hrefs, which the renderer deliberately keeps", () => {
    // These survive `isAllowedLinkHref` with an intact href AND a data-external
    // tag, so neither the renderer nor the `a[data-external]` selector is the
    // guard here. Asserted through the real renderer so the two stay in step.
    for (const href of ["/Applications/Calculator.app", "/etc/passwd", "#anchor", "?q=1"]) {
      expect(renderMarkdown(`[x](${href})`)).toContain('data-external="true"');
      expect(isOpenableHref(href)).toBe(false);
    }
  });

  test("refuses custom and dangerous schemes", () => {
    expect(isOpenableHref("file:///etc/passwd")).toBe(false);
    expect(isOpenableHref("javascript:alert(1)")).toBe(false);
    expect(isOpenableHref("someapp://boom")).toBe(false);
    expect(isOpenableHref("//evil.example.com")).toBe(false);
  });

  test("refuses nothing at all", () => {
    expect(isOpenableHref(null)).toBe(false);
    expect(isOpenableHref(undefined)).toBe(false);
    expect(isOpenableHref("")).toBe(false);
  });
});
