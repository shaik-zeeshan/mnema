<script lang="ts">
  // A failed Chat turn: one kit notice (chat.html "Turn failures") — a title,
  // one plain sentence and the ONE action that can help (Direction A,
  // CH-02/CH-03). The message may be a raw resolve code (a pre-flight failure)
  // or a sentence with a `kind`; `turnErrorCopy` maps both, so no code ever
  // reaches the screen. The action shows only on the trailing turn — send()
  // derives turnIndex from turns.length, so acting on a mid-thread error would
  // collide indexes.
  import ChatgptConnect from "$lib/components/ChatgptConnect.svelte";
  import { openSettings } from "$lib/surface-windows";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";
  import { turnErrorCopy } from "$lib/insights/engine-state";
  import type { TurnErrorKind } from "$lib/insights/conversation";
  import type { AiRuntimeSettings } from "$lib/types/recording";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCloudOff from "~icons/lucide/cloud-off";
  import IconRetry from "~icons/lucide/rotate-ccw";

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
  // Offline/unreachable never reads as a sign-in problem (ADR 0058): its own
  // cloud-off warn notice, like chat.html's `offline` state.
  const unreachable = $derived(kind === "unreachable" || /unreachable/.test(message ?? ""));
  const head = $derived.by((): { tone: "warn" | "danger"; title: string } => {
    if (copy.action === "reconnect") return { tone: "danger", title: "Sign in again to keep chatting" };
    if (unreachable) return { tone: "warn", title: "Can’t reach the AI provider right now" };
    if (kind === "quota") return { tone: "warn", title: "The provider is limiting requests" };
    if (kind === "context_too_long") return { tone: "warn", title: "This chat is too long for the model" };
    if (copy.action === "settings") return { tone: "danger", title: "The AI settings need a look" };
    return { tone: "danger", title: "Couldn’t answer this one" };
  });
</script>

<div class="mx-notice" data-tone={head.tone} role={head.tone === "danger" ? "alert" : "status"}>
  <span class="mx-notice__icon">
    {#if unreachable}<IconCloudOff width="15" height="15" />{:else}<IconAlert width="15" height="15" />{/if}
  </span>
  <b class="mx-notice__title">{head.title}</b>
  <p class="mx-notice__text">{copy.text}</p>
  {#if trailing}
    <div class="mx-notice__acts">
      {#if copy.action === "reconnect" && copy.reconnectProviderId}
        <!-- Retry can never work until the user signs in, so it isn't offered. -->
        <ChatgptConnect
          providerId={copy.reconnectProviderId}
          connected={false}
          signInLabel="Sign in again"
          onchange={() => {}}
        />
      {:else if copy.action === "settings"}
        <button type="button" class="mx-btn mx-btn--sm" onclick={() => void openSettings("intelligence")}>
          Open settings
        </button>
      {:else if copy.action === "new_chat"}
        <button type="button" class="mx-btn mx-btn--sm" onclick={() => conversationStore.requestNewChat()}>
          New chat
        </button>
      {:else}
        <!-- Re-issue the same question. The composer is also restored with the
             question, so this and a manual edit-and-resend both work. -->
        <button type="button" class="mx-btn mx-btn--sm" disabled={retryDisabled} onclick={onRetry}>
          <IconRetry width="13" height="13" aria-hidden="true" />Retry
        </button>
      {/if}
    </div>
  {/if}
</div>
