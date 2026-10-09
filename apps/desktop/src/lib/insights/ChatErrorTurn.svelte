<script lang="ts">
  // A failed Chat turn: one plain sentence plus the ONE action that can help
  // (Direction A, CH-02/CH-03). The message may be a raw resolve code (a
  // pre-flight failure) or a sentence with a `kind`; `turnErrorCopy` maps both,
  // so no code ever reaches the screen. The action shows only on the trailing
  // turn — send() derives turnIndex from turns.length, so acting on a
  // mid-thread error would collide indexes.
  import ChatgptConnect from "$lib/components/ChatgptConnect.svelte";
  import { openSettings } from "$lib/surface-windows";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";
  import { turnErrorCopy } from "$lib/insights/engine-state";
  import type { TurnErrorKind } from "$lib/insights/conversation";
  import type { AiRuntimeSettings } from "$lib/types/recording";

  interface Props {
    message: string | null;
    kind: TurnErrorKind | null;
    settings: AiRuntimeSettings | null;
    trailing: boolean;
    retryDisabled: boolean;
    onRetry: () => void;
  }

  let { message, kind, settings, trailing, retryDisabled, onRetry }: Props = $props();

  const copy = $derived(turnErrorCopy(message, kind, settings));
</script>

<div class="turn-error" role="alert">
  <p class="state state--error">{copy.text}</p>
  {#if trailing}
    {#if copy.action === "reconnect" && copy.reconnectProviderId}
      <!-- Retry can never work until the user signs in, so it isn't offered. -->
      <ChatgptConnect
        providerId={copy.reconnectProviderId}
        connected={false}
        signInLabel="Sign in again"
        onchange={() => {}}
      />
    {:else if copy.action === "settings"}
      <button type="button" class="turn-retry" onclick={() => void openSettings("intelligence")}>
        Open settings
      </button>
    {:else if copy.action === "new_chat"}
      <button type="button" class="turn-retry" onclick={() => conversationStore.requestNewChat()}>
        New chat
      </button>
    {:else}
      <!-- Re-issue the same question. The composer is also restored with the
           question, so this and a manual edit-and-resend both work. -->
      <button type="button" class="turn-retry" disabled={retryDisabled} onclick={onRetry}>
        <span class="turn-retry-ico" aria-hidden="true">↻</span>
        Retry
      </button>
    {/if}
  {/if}
</div>

<style>
  .state {
    margin: 0;
    font-size: 12px;
    line-height: 1.55;
  }
  .state--error {
    color: var(--app-danger);
  }
  .turn-error {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
  }
  .turn-retry {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: inherit;
    font-size: 11px;
    padding: 4px 11px;
    border: 1px solid var(--app-danger-border);
    border-radius: 7px;
    background: var(--app-danger-bg);
    color: var(--app-danger-text);
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease,
      opacity 0.12s ease;
  }
  .turn-retry:hover:not(:disabled) {
    border-color: var(--app-danger);
  }
  .turn-retry:focus-visible {
    outline: none;
    box-shadow: var(--app-ring-danger);
  }
  .turn-retry:not(:disabled):active {
    transform: translateY(1px);
  }
  .turn-retry:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .turn-retry-ico {
    font-size: 12px;
    line-height: 1;
  }
</style>
