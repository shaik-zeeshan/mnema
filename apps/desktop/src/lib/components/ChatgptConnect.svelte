<script lang="ts">
  // Connect flow for a `chatgpt` provider instance (OAuth device-code login) —
  // the one cloud kind with no API key field. Shared by Settings and
  // onboarding, so it is self-contained: it invokes the login command, shows
  // the user code, listens for the terminal `chatgpt_login_update` event, and
  // reports outcomes up through `onchange` so each surface can refresh its own
  // presence/readiness state.
  //
  // Usage counts against the user's own ChatGPT Plus/Pro plan; the token set
  // lands in the encrypted vault (same slot an API key would occupy), so
  // "disconnect" is the existing clear-provider-key command.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { humanizeError } from "$lib/format-error";

  let {
    providerId,
    connected,
    onchange,
    autostart = false,
  }: {
    providerId: string;
    /** Does the vault hold a token set for this instance (parent-owned)? */
    connected: boolean;
    /** Fired after a login completes or a disconnect lands. */
    onchange: () => void;
    /**
     * Begin the login immediately on mount (used by onboarding, where the
     * provider instance is created by the same click that wants the login).
     * Ignored when already connected.
     */
    autostart?: boolean;
  } = $props();

  interface LoginPrompt {
    userCode: string;
    verifyUrl: string;
  }
  interface LoginUpdate {
    providerId: string;
    connected: boolean;
    error?: string;
  }

  type Phase = { kind: "idle" } | { kind: "starting" } | { kind: "waiting"; prompt: LoginPrompt };
  let phase = $state<Phase>({ kind: "idle" });
  let error = $state<string | null>(null);
  let disconnecting = $state(false);

  onMount(() => {
    if (autostart && !connected) void beginLogin();
    let unlisten: (() => void) | null = null;
    let destroyed = false;
    void listen<LoginUpdate>("chatgpt_login_update", (event) => {
      if (event.payload.providerId !== providerId) return;
      phase = { kind: "idle" };
      error = event.payload.connected ? null : (event.payload.error ?? "Sign-in failed.");
      if (event.payload.connected) onchange();
    }).then((fn) => {
      if (destroyed) fn();
      else unlisten = fn;
    });
    return () => {
      destroyed = true;
      unlisten?.();
    };
  });

  async function beginLogin(): Promise<void> {
    error = null;
    phase = { kind: "starting" };
    try {
      const prompt = await invoke<LoginPrompt>("ai_runtime_chatgpt_begin_login", {
        request: { provider: providerId },
      });
      phase = { kind: "waiting", prompt };
      // Open the verify page right away — the code stays visible here to type.
      void openUrl(prompt.verifyUrl).catch((e) =>
        console.error("[ChatgptConnect] open verify url failed", e),
      );
    } catch (e) {
      phase = { kind: "idle" };
      error = humanizeError(e);
    }
  }

  async function disconnect(): Promise<void> {
    error = null;
    try {
      const confirmed = await confirm(
        "Disconnecting removes the ChatGPT sign-in token from the vault right away. Any AI feature using this provider stops working until you sign in again.",
        {
          title: "Disconnect ChatGPT?",
          kind: "warning",
          okLabel: "Disconnect",
          cancelLabel: "Stay Connected",
        },
      );
      if (!confirmed) return;
    } catch {
      // A dialog failure must not silently delete the token — bail.
      return;
    }
    disconnecting = true;
    try {
      await invoke("ai_runtime_clear_provider_key", { request: { provider: providerId } });
      onchange();
    } catch (e) {
      error = humanizeError(e);
    } finally {
      disconnecting = false;
    }
  }

  function copyCode(code: string): void {
    void navigator.clipboard?.writeText(code).catch(() => {});
  }
</script>

<div class="chatgpt-connect">
  {#if phase.kind === "waiting"}
    <div class="chatgpt-connect__code-block">
      <p class="chatgpt-connect__hint">
        Enter this code at
        <button
          type="button"
          class="chatgpt-connect__link"
          onclick={() => phase.kind === "waiting" && void openUrl(phase.prompt.verifyUrl)}
        >auth.openai.com/codex/device</button> — waiting for approval…
      </p>
      <button
        type="button"
        class="chatgpt-connect__code"
        title="Click to copy"
        onclick={() => phase.kind === "waiting" && copyCode(phase.prompt.userCode)}
      >{phase.prompt.userCode}</button>
      <div class="chatgpt-connect__actions">
        <button type="button" class="chatgpt-connect__btn" onclick={beginLogin}>Start over</button>
      </div>
    </div>
  {:else}
    <div class="chatgpt-connect__actions">
      <button
        type="button"
        class="chatgpt-connect__btn chatgpt-connect__btn--primary"
        disabled={phase.kind === "starting" || disconnecting}
        aria-busy={phase.kind === "starting"}
        onclick={beginLogin}
      >
        {phase.kind === "starting"
          ? "Contacting OpenAI…"
          : connected
            ? "Reconnect ChatGPT"
            : "Connect ChatGPT"}
      </button>
      {#if connected}
        <button
          type="button"
          class="chatgpt-connect__btn"
          disabled={disconnecting || phase.kind !== "idle"}
          onclick={disconnect}
        >
          {disconnecting ? "Disconnecting…" : "Disconnect"}
        </button>
      {/if}
    </div>
    <p class="chatgpt-connect__hint">
      Signs in with your own ChatGPT Plus/Pro account in the browser — no API key. Usage counts
      against your plan's Codex limits. The sign-in token is stored only in the encrypted vault.
    </p>
  {/if}
  {#if error}
    <p class="chatgpt-connect__error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .chatgpt-connect {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
  }

  .chatgpt-connect__actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .chatgpt-connect__btn {
    appearance: none;
    border: 1px solid var(--app-border, rgba(127, 127, 127, 0.35));
    background: transparent;
    color: var(--app-text, inherit);
    border-radius: 7px;
    padding: 5px 12px;
    font-size: var(--text-sm, 13px);
    cursor: pointer;
  }

  .chatgpt-connect__btn:disabled {
    opacity: var(--app-disabled-opacity, 0.5);
    cursor: default;
  }

  .chatgpt-connect__btn--primary {
    font-weight: 600;
  }

  .chatgpt-connect__code-block {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* The code is the hero while waiting: big, monospace, one click to copy. */
  .chatgpt-connect__code {
    appearance: none;
    border: 1px dashed var(--app-border, rgba(127, 127, 127, 0.35));
    background: transparent;
    color: var(--app-text, inherit);
    border-radius: 8px;
    padding: 10px 14px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 1.35em;
    letter-spacing: 0.12em;
    text-align: center;
    cursor: copy;
    align-self: flex-start;
  }

  .chatgpt-connect__link {
    appearance: none;
    border: none;
    background: none;
    padding: 0;
    color: inherit;
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }

  .chatgpt-connect__hint {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: var(--app-text-muted, rgba(127, 127, 127, 0.9));
  }

  .chatgpt-connect__error {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: var(--app-danger, #c0392b);
  }
</style>
