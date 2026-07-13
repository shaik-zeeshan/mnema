<script lang="ts">
  // McpOAuthConnect — the in-modal OAuth connect experience (Plan: MCP OAuth,
  // slice 8b; mockup oauth-connectors.html `oauthStageHTML`). Presentational: it
  // renders exactly one of the four stages from the `stage` prop and calls back.
  // The stage is DERIVED upstream (in McpConnectorPicker) from the live store
  // state — this component owns no lifecycle, no timers. Its parent renders the
  // lede + chips; this is just the stage block.
  import IconExternal from "~icons/lucide/external-link";
  import IconCheck from "~icons/lucide/check";
  import IconAlert from "~icons/lucide/triangle-alert";
  import type { McpOAuthStage } from "$lib/settings/state/mcp-oauth-stage";

  interface Props {
    stage: McpOAuthStage;
    /** Service name, for the stage copy ("Approve Notion in your browser"). */
    label: string;
    /** "Connect" (fresh) or "Reconnect" (token expired) — the idle button verb. */
    verb?: "Connect" | "Reconnect";
    /** Tool count for the authorized line, if known ("Connected · 7 tools"). */
    tools?: number | null;
    onConnect: () => void;
    onCancel: () => void;
    onDone: () => void;
    onRetry: () => void;
  }

  let { stage, label, verb = "Connect", tools = null, onConnect, onCancel, onDone, onRetry }: Props =
    $props();

  const who = $derived(label.trim() || "access");
</script>

<div class="oauth">
  {#if stage === "idle"}
    <div class="oauth__stage">
      <button class="btn btn--primary oauth__connect" type="button" onclick={onConnect}>
        {verb}{label.trim() ? ` ${label.trim()}` : " with OAuth"}
      </button>
      <p class="oauth__reassure">
        Opens your browser to approve. Nothing is pasted here — only the returned token lands in your
        system keychain.
      </p>
    </div>
  {:else if stage === "authorizing"}
    <div class="oauth__stage">
      <span class="oauth__glyph"><IconExternal aria-hidden="true" /></span>
      <p class="oauth__ok">Approve {who} in your browser</p>
      <p class="oauth__sub">
        We opened your browser — approve access there, then you'll come right back.
      </p>
      <div class="oauth__waiting"><span class="oauth__pulse" aria-hidden="true"></span>Waiting…</div>
      <!-- ponytail: no cancel command — the backend pending entry lapses on its
           own (~5 min TTL, ADR 0051). Cancel just dismisses the modal; the row
           keeps "authorizing…" until the callback resolves or the next status
           refresh reverts it. -->
      <button class="btn btn--ghost btn--sm" type="button" onclick={onCancel}>Cancel</button>
    </div>
  {:else if stage === "authorized"}
    <div class="oauth__stage">
      <span class="oauth__check"><IconCheck aria-hidden="true" /></span>
      <p class="oauth__ok">Connected{tools != null ? ` · ${tools} tools` : ""}</p>
      <button class="btn btn--primary oauth__done" type="button" onclick={onDone}>Done</button>
    </div>
  {:else}
    <div class="oauth__stage">
      <span class="oauth__x"><IconAlert aria-hidden="true" /></span>
      <p class="oauth__ok">Approval didn't go through</p>
      <p class="oauth__sub">You cancelled or denied access in the browser. Nothing was stored.</p>
      <button class="btn btn--primary oauth__retry" type="button" onclick={onRetry}>Try again</button>
    </div>
  {/if}
</div>

<style>
  /* Ported verbatim (tokens + shape) from the mockup's `.oauth*` stage CSS. */
  .oauth {
    display: flex;
    flex-direction: column;
  }
  .oauth__stage {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 12px;
    padding: 26px 8px 12px;
  }
  .oauth__connect.btn {
    padding: 11px 24px;
    font-size: 12px;
    letter-spacing: 0.06em;
  }
  .oauth__reassure {
    margin: 0;
    max-width: 360px;
    font-size: 10.5px;
    line-height: 1.5;
    color: var(--app-text-subtle);
  }
  .oauth__ok {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .oauth__sub {
    margin: -4px 0 0;
    max-width: 340px;
    font-size: 10.5px;
    line-height: 1.5;
    color: var(--app-text-subtle);
  }

  /* browser-handoff waiting stage */
  .oauth__glyph {
    width: 46px;
    height: 46px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    color: var(--app-accent);
    box-shadow: 0 0 18px var(--app-accent-glow);
    animation: oauth-in 0.2s ease-out;
  }
  .oauth__glyph :global(svg) {
    width: 22px;
    height: 22px;
  }
  .oauth__waiting {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .oauth__pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--app-accent);
    box-shadow: 0 0 8px var(--app-accent-glow);
    animation: oauth-pulse 1.4s ease-in-out infinite;
  }

  /* authorized stage */
  .oauth__check {
    width: 36px;
    height: 36px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    color: var(--app-accent);
    box-shadow: 0 0 18px var(--app-accent-glow);
    animation: oauth-in 0.2s ease-out;
  }
  .oauth__check :global(svg) {
    width: 20px;
    height: 20px;
    stroke-width: 2.5;
  }

  /* denied / error stage */
  .oauth__x {
    width: 36px;
    height: 36px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--app-danger-bg);
    border: 1px solid var(--app-danger-border);
    color: var(--app-danger);
    animation: oauth-in 0.2s ease-out;
  }
  .oauth__x :global(svg) {
    width: 20px;
    height: 20px;
  }

  @keyframes oauth-in {
    from {
      transform: scale(0.4);
      opacity: 0;
    }
    to {
      transform: scale(1);
      opacity: 1;
    }
  }
  @keyframes oauth-pulse {
    0%,
    100% {
      opacity: 0.3;
      transform: scale(0.8);
    }
    50% {
      opacity: 1;
      transform: scale(1);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .oauth__glyph,
    .oauth__check,
    .oauth__x {
      animation: none;
    }
    .oauth__pulse {
      animation: none;
      opacity: 1;
    }
  }
</style>
