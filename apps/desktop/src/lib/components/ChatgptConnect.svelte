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
  import IconCopy from "~icons/lucide/copy";
  import IconCheck from "~icons/lucide/check";

  let {
    providerId,
    connected,
    onchange,
    autostart = false,
  }: {
    providerId: string;
    /** Does the vault hold a token set for this instance (parent-owned)? */
    connected: boolean;
    /**
     * Fired after a disconnect lands. NOT fired for a completed sign-in: the
     * parent surface listens for `chatgpt_login_update` itself, because this
     * component can be unmounted while the backend poll is still running.
     */
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
    // Local phase/error only. The REFRESH on a landed sign-in is deliberately
    // NOT ours: the backend poll runs for up to 15 minutes and this component
    // is unmounted by an ordinary interaction long before that (toggling the
    // onboarding kind select, collapsing the Settings row, "Done" on a card),
    // which would drop the outcome on the floor. The parent surface — which
    // outlives every one of those — owns `chatgpt_login_update` and refreshes
    // from there. `onchange` is for the disconnect below, which is always
    // user-initiated and therefore always mounted.
    void listen<LoginUpdate>("chatgpt_login_update", (event) => {
      if (event.payload.providerId !== providerId) return;
      phase = { kind: "idle" };
      error = event.payload.connected ? null : (event.payload.error ?? "Sign-in failed.");
    }).then((fn) => {
      if (destroyed) fn();
      else unlisten = fn;
    });
    return () => {
      destroyed = true;
      gone = true;
      unlisten?.();
      // The "Copied" flash is scheduled work owned by this instance: left
      // armed it writes to destroyed state 1.5s after the row is gone.
      if (copiedTimer) clearTimeout(copiedTimer);
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

  /**
   * Give up on a pending login.
   *
   * The code UI otherwise has no exit but the backend's 15-minute timeout, and
   * "Start over" just arms a second poll. Cancelling tells the backend too:
   * an abandoned poll keeps hitting `auth.openai.com`, and that endpoint treats
   * a rate limit as terminal — so the login the user gives up on can be what
   * kills the next one they actually want.
   */
  async function cancelLogin(): Promise<void> {
    phase = { kind: "idle" };
    error = null;
    try {
      await invoke("ai_runtime_chatgpt_cancel_login", { request: { provider: providerId } });
    } catch (e) {
      // The UI is already back to idle; a failed cancel just leaves the poll to
      // time out on its own, which is what used to happen every time.
      console.error("[ChatgptConnect] cancel login failed", e);
    }
  }

  async function disconnect(): Promise<void> {
    error = null;
    // Arm the latch BEFORE the awaited dialog, not after: the Disconnect button
    // reads `disconnecting`, so leaving it false for the whole confirm lets a
    // second click open a second dialog and fire a second revocation (which
    // also cancels a login the first one's `cancel_login` already re-armed).
    // Same rule the store's `clearAiProviderKey` states for the same command.
    if (disconnecting) return;
    disconnecting = true;
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
      if (!confirmed) {
        disconnecting = false;
        return;
      }
    } catch {
      // A dialog failure must not silently delete the token — bail.
      disconnecting = false;
      return;
    }
    try {
      await invoke("ai_runtime_clear_provider_key", { request: { provider: providerId } });
      onchange();
    } catch (e) {
      error = humanizeError(e);
    } finally {
      disconnecting = false;
    }
  }

  let copied = $state(false);
  let copiedTimer: ReturnType<typeof setTimeout> | null = null;
  // Set by the teardown above. `copyCode` awaits the clipboard write, so it can
  // resolve after the row is gone — and arming the flash timer there would
  // schedule work with no teardown left to clear it.
  let gone = false;

  async function copyCode(code: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(code);
    } catch (e) {
      // A rejected write must not claim success — the user would paste nothing
      // into the OpenAI page with no signal. The code stays selectable.
      console.error("[ChatgptConnect] copy failed", e);
      return;
    }
    if (gone) return;
    copied = true;
    if (copiedTimer) clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => (copied = false), 1500);
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
      <div class="chatgpt-connect__code-row">
        <button
          type="button"
          class="chatgpt-connect__code"
          title="Click to copy"
          onclick={() => phase.kind === "waiting" && void copyCode(phase.prompt.userCode)}
        >{phase.prompt.userCode}</button>
        <button
          type="button"
          class="chatgpt-connect__btn chatgpt-connect__copy"
          title={copied ? "Copied" : "Copy code"}
          aria-label={copied ? "Copied" : "Copy code"}
          onclick={() => phase.kind === "waiting" && void copyCode(phase.prompt.userCode)}
        >
          {#if copied}<IconCheck />{:else}<IconCopy />{/if}
        </button>
      </div>
      <div class="chatgpt-connect__actions">
        <button type="button" class="chatgpt-connect__btn" onclick={beginLogin}>Start over</button>
        <button type="button" class="chatgpt-connect__btn" onclick={() => void cancelLogin()}>
          Cancel
        </button>
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

  .chatgpt-connect__code-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .chatgpt-connect__copy {
    display: inline-flex;
    align-items: center;
    padding: 8px;
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
