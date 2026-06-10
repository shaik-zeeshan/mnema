<script lang="ts">
  // AnswerProse — the single Markdown body renderer ("Answer Prose") shared by
  // both Ask AI surfaces (Quick Recall + Insights Chat). It owns the container,
  // the rendered HTML (via `renderMarkdown`), a delegated click handler (copy
  // buttons + external-link routing), per-instance render memoization, and all
  // the scoped prose / code-chrome / highlight.js styling keyed to the app's
  // terminal/green palette.
  //
  // Highlighting is deferred while streaming: a live turn renders plain markdown
  // (cheap, no token spans), and re-renders ONCE with full highlight.js coloring
  // when the turn settles (`isStreaming` flips to false).
  import { onDestroy } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { renderMarkdown } from "$lib/markdown";

  let {
    source,
    isStreaming = false,
    onOpenLink,
  }: {
    source: string;
    isStreaming?: boolean;
    onOpenLink?: (href: string) => void;
  } = $props();

  // The rendered HTML, memoized to its (source, isStreaming) cache key: $derived
  // recomputes only when one of those tracked inputs changes, and lives per
  // instance so each turn renders independently. While streaming we pass
  // `highlight: false` (plain, escaped code); once settled we re-render once with
  // full highlighting.
  //
  // SECURITY: this HTML is trusted ONLY because `markdown.ts` is the sanctioned
  // hardening boundary (`html: false`, image strip, link allowlist, highlight.js
  // escaping). This component must not introduce any other unescaped injection.
  let html = $derived(
    source.length > 0 ? renderMarkdown(source, { highlight: !isStreaming }) : "",
  );

  // Per-instance timer for the copy button's transient "Copied" label, cleared
  // onDestroy so a swapped-back revert never fires against a torn-down element.
  let copyRevertTimer: ReturnType<typeof setTimeout> | null = null;

  // Delegated click handler over the rendered HTML. Two responsibilities, checked
  // in order: a code-block copy button, then an external link.
  function handleClick(event: MouseEvent): void {
    const target = event.target as HTMLElement | null;

    // 1. Copy button — read the code as-rendered and flash a "Copied" label.
    const btn = target?.closest("[data-copy-code]") as HTMLButtonElement | null;
    if (btn) {
      const block = btn.closest(".answer-code");
      const codeEl = block?.querySelector("pre") ?? block?.querySelector("code");
      const text = codeEl?.textContent ?? "";
      void navigator.clipboard.writeText(text);
      // Mutate the button DOM directly so the whole component never re-renders
      // for a transient affordance. Guard the revert against teardown.
      btn.textContent = "Copied";
      if (copyRevertTimer !== null) clearTimeout(copyRevertTimer);
      copyRevertTimer = setTimeout(() => {
        copyRevertTimer = null;
        if (btn.isConnected) btn.textContent = "Copy";
      }, 1200);
      return;
    }

    // 2. External link — route through the surface callback (Quick Recall must
    //    suppress its panel blur-dismiss first) or open it directly.
    const anchor = target?.closest("a[data-external]") as HTMLAnchorElement | null;
    if (anchor) {
      event.preventDefault();
      const href = anchor.getAttribute("href");
      if (href !== null && href.length > 0) {
        if (onOpenLink) onOpenLink(href);
        else void openUrl(href);
      }
    }
  }

  onDestroy(() => {
    if (copyRevertTimer !== null) clearTimeout(copyRevertTimer);
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="answer-prose" class:is-streaming={isStreaming} onclick={handleClick}>
  {@html html}
</div>

<style>
  /* All selectors are scoped under `.answer-prose`. Descendants of the injected
     {@html} content need `:global(...)`, since Svelte prunes scoped styles that
     don't appear in the component's own template markup. Every color comes from
     the app theme vars (which flip dark/light via `data-theme`) — never hex. */
  .answer-prose {
    font-size: 13px;
    color: var(--app-text);
    line-height: 1.6;
    word-break: break-word;
    overflow-wrap: anywhere;
  }
  .answer-prose :global(> *:first-child) {
    margin-top: 0;
  }
  .answer-prose :global(> *:last-child) {
    margin-bottom: 0;
  }

  /* ── Prose elements ──────────────────────────────────────────────────── */
  .answer-prose :global(p),
  .answer-prose :global(ul),
  .answer-prose :global(ol),
  .answer-prose :global(blockquote),
  .answer-prose :global(table) {
    margin: 0 0 0.7em;
  }

  .answer-prose :global(h1),
  .answer-prose :global(h2),
  .answer-prose :global(h3),
  .answer-prose :global(h4),
  .answer-prose :global(h5),
  .answer-prose :global(h6) {
    margin: 1em 0 0.45em;
    line-height: 1.3;
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .answer-prose :global(h1) {
    font-size: 1.3em;
  }
  .answer-prose :global(h2) {
    font-size: 1.18em;
  }
  .answer-prose :global(h3) {
    font-size: 1.06em;
  }
  .answer-prose :global(h4),
  .answer-prose :global(h5),
  .answer-prose :global(h6) {
    font-size: 1em;
  }

  .answer-prose :global(strong) {
    font-weight: 600;
    color: var(--app-text-strong);
  }

  .answer-prose :global(a) {
    color: var(--app-accent);
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
  }

  .answer-prose :global(ul),
  .answer-prose :global(ol) {
    padding-left: 1.4em;
  }
  .answer-prose :global(li) {
    margin: 0.2em 0;
  }
  .answer-prose :global(li::marker) {
    color: var(--app-text-muted);
  }
  .answer-prose :global(li > ul),
  .answer-prose :global(li > ol) {
    margin: 0.2em 0;
  }

  /* Inline code — explicitly NOT the code inside `.answer-code` blocks (those are
     re-styled below). */
  .answer-prose :global(code:not(.answer-code code)) {
    font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas,
      monospace;
    font-size: 0.88em;
    padding: 0.1em 0.35em;
    border-radius: 4px;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    color: var(--app-text-strong);
  }

  .answer-prose :global(blockquote) {
    padding: 0.1em 0 0.1em 0.9em;
    border-left: 2px solid var(--app-accent-border);
    color: var(--app-text-muted);
  }

  .answer-prose :global(table) {
    border-collapse: collapse;
    font-size: 11.5px;
  }
  .answer-prose :global(th),
  .answer-prose :global(td) {
    border: 1px solid var(--app-border);
    padding: 4px 8px;
    text-align: left;
  }

  /* ── Code-block chrome (emitted by markdown.ts) ──────────────────────── */
  .answer-prose :global(.answer-code) {
    margin: 0 0 0.7em;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    overflow: hidden;
    background: var(--app-surface-subtle);
  }
  .answer-prose :global(.answer-code__header) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 5px 10px;
    border-bottom: 1px solid var(--app-border);
    background: var(--app-surface-raised);
  }
  .answer-prose :global(.answer-code__lang) {
    font-size: 10px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    font-family: ui-monospace, monospace;
  }
  .answer-prose :global(.answer-code__copy) {
    font: inherit;
    font-size: 10.5px;
    padding: 2px 8px;
    border: 1px solid var(--app-border);
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition: color 0.12s ease, border-color 0.12s ease;
  }
  .answer-prose :global(.answer-code__copy:hover) {
    color: var(--app-text-strong);
    border-color: var(--app-border-strong);
  }
  .answer-prose :global(.answer-code__copy:active) {
    color: var(--app-accent);
  }
  .answer-prose :global(.answer-code__pre) {
    margin: 0;
    padding: 10px 12px;
    overflow-x: auto;
    background: transparent;
  }
  .answer-prose :global(.answer-code__pre code) {
    font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas,
      monospace;
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--app-text);
    padding: 0;
    border: none;
    background: none;
  }

  /* ── highlight.js token theme ────────────────────────────────────────── */
  /* Mapped to the palette for a calm terminal/green feel — no stock hljs
     stylesheet is imported. The `pre code` color above is the default token. */
  .answer-prose :global(.hljs-keyword),
  .answer-prose :global(.hljs-built_in),
  .answer-prose :global(.hljs-literal) {
    color: var(--cat-communication);
  }
  .answer-prose :global(.hljs-string),
  .answer-prose :global(.hljs-meta-string),
  .answer-prose :global(.hljs-regexp) {
    color: var(--app-source-mic);
  }
  .answer-prose :global(.hljs-number) {
    color: var(--cat-personal);
  }
  .answer-prose :global(.hljs-comment),
  .answer-prose :global(.hljs-quote) {
    color: var(--app-text-subtle);
    font-style: italic;
  }
  .answer-prose :global(.hljs-title),
  .answer-prose :global(.hljs-title.function_),
  .answer-prose :global(.hljs-function .hljs-title),
  .answer-prose :global(.hljs-section) {
    color: var(--cat-research);
  }
  .answer-prose :global(.hljs-attr),
  .answer-prose :global(.hljs-attribute),
  .answer-prose :global(.hljs-property) {
    color: var(--cat-learning);
  }
  .answer-prose :global(.hljs-variable),
  .answer-prose :global(.hljs-template-variable),
  .answer-prose :global(.hljs-params) {
    color: var(--app-text);
  }
  .answer-prose :global(.hljs-type),
  .answer-prose :global(.hljs-class .hljs-title),
  .answer-prose :global(.hljs-title.class_) {
    color: var(--cat-learning);
  }
  .answer-prose :global(.hljs-tag),
  .answer-prose :global(.hljs-name),
  .answer-prose :global(.hljs-selector-tag) {
    color: var(--cat-communication);
  }
  .answer-prose :global(.hljs-symbol),
  .answer-prose :global(.hljs-bullet),
  .answer-prose :global(.hljs-meta) {
    color: var(--app-text-muted);
  }
  .answer-prose :global(.hljs-deletion) {
    color: var(--app-danger);
  }
  .answer-prose :global(.hljs-addition) {
    color: var(--app-accent);
  }
  .answer-prose :global(.hljs-emphasis) {
    font-style: italic;
  }
  .answer-prose :global(.hljs-strong) {
    font-weight: 600;
  }

  /* ── Streaming caret ─────────────────────────────────────────────────── */
  .answer-prose.is-streaming::after {
    content: "▍";
    color: var(--app-accent);
    animation: answer-caret 1s step-end infinite;
  }
  @keyframes answer-caret {
    50% {
      opacity: 0;
    }
  }
</style>
