<!--
  Onboarding AI setup (issue #195, slice 11).

  Skin of record: `docs/mockups/redesign/onboarding.html` (Change settings → AI
  features): the fork is two `mx-choice` tiles, then a kit-control connect row,
  the data-path wire, the scan table and one `mx-panel` per connection.

  Three things it changes about the shipping surface:
   · "Later" is the PRIMARY path. Skipping is a first-class informed choice that
     names the three features that stay dark and the exact Settings path that
     turns them on.
   · There is no Save button. Reaching the provider IS the save: the key goes to
     the vault, the model list comes back, and that list is the receipt. A key
     that cannot list models never becomes a usable provider.
   · Only models a connected engine listed JUST NOW are offerable — the chips in
     each live card are the selection surface, and the readout below repeats them.

  Secret handling: the key field is type="password", has no `value` attribute and
  is never bound to state. The string is read from the DOM node once, handed to
  the vault, and the field is cleared — nothing renders it, so it reaches no
  markup, no aria-label, no title and no URL.

  Motion: none ambient; a kit spinner only while a request is in flight.
-->
<script lang="ts">
  import { tick, untrack, type Snippet } from "svelte";
  import ChatgptConnect from "$lib/components/ChatgptConnect.svelte";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCheck from "~icons/lucide/check";
  import IconChevron from "~icons/lucide/chevron-down";
  import IconSearch from "~icons/lucide/search";
  import {
    AI_LOCAL_DEFAULT_ENDPOINTS,
    aiProviderKindLabel,
    baseUrlHost,
    isCloudAiProviderKind,
  } from "$lib/settings/state/ai-providers";
  import { aiVerificationWord, plural, type AiVerification } from "$lib/onboarding/ai-readiness";
  import type { AiEngineRef, AiProviderConfig, AiProviderKind } from "$lib/types";

  /**
   * The onboarding AI store's surface, declared structurally so this component
   * doesn't reach into `routes/`. `createOnboardingAiStore()` satisfies it.
   */
  interface AiSetupStore {
    draftAiProviders: AiProviderConfig[];
    draftAiDefaultModel: AiEngineRef | null;
    aiVerifications: Record<string, AiVerification>;
    /** Which cloud instances have a key in the vault, by instance id. */
    aiProviderKeySaved: Record<string, boolean>;
    aiConfigMissing: string | null;
    aiConfigReady: boolean;
    aiProviderRemoving: boolean;
    addProvider: (kind: AiProviderKind, baseUrl?: string) => string | null;
    removeProvider: (id: string) => Promise<void>;
    verifyProvider: (id: string) => Promise<void>;
    invalidateVerification: (id: string) => void;
    /** Re-probe the vault credential, then re-verify — after a ChatGPT login. */
    handleChatgptChanged: (id: string) => Promise<void>;
    saveKeyAndVerify: (id: string, key: string) => Promise<void>;
    probeEndpoint: (
      kind: AiProviderKind,
      baseUrl: string,
    ) => Promise<{ models: string[]; latencyMs: number } | null>;
    aiProviderInstanceLabel: (provider: AiProviderConfig) => string;
  }

  let {
    ai,
    aiEnabled,
    onToggleAi,
    notices,
  }: {
    ai: AiSetupStore;
    aiEnabled: boolean;
    onToggleAi: (on: boolean) => void;
    /** The caller's in-place warnings, drawn between the lead and the fork. */
    notices?: Snippet;
  } = $props();

  // ── The ports a local scan asks about ─────────────────────────────────────
  // The two default endpoints plus the port LM Studio and friends serve an
  // OpenAI-compatible API on. Nothing else is guessed.
  const SCAN_TARGETS: { port: number; kind: AiProviderKind; name: string }[] = [
    { port: 11434, kind: "ollama", name: "Ollama" },
    { port: 8080, kind: "llamafile", name: "Llamafile" },
    { port: 1234, kind: "llamafile", name: "OpenAI-compatible server" },
  ];
  const LOCAL_KINDS: AiProviderKind[] = ["ollama", "llamafile"];
  const CLOUD_KINDS: AiProviderKind[] = ["anthropic", "openai", "chatgpt", "openai_compatible"];

  const VAULT_NOTE =
    "The key goes into the app's encrypted vault (day.mnema.vault, unlocked by one keychain item) — " +
    "never a config file, and this window forgets the string the moment it is sent.";

  // ── Local state (presentation only; every durable value lives in the store) ─
  type Mode = null | "later" | "now";
  // The fork is a QUESTION, and a connected engine has already answered it. This
  // component remounts on every *Your settings* ⇄ *Change settings* round trip
  // while the store keeps the providers, so starting at `null` unconditionally
  // hides an engine the user connected a moment ago behind "set it up later".
  let mode = $state<Mode>(untrack(() => ai.draftAiProviders.length) > 0 ? "now" : null);
  let kind = $state<AiProviderKind>("ollama");
  let compatUrl = $state("");
  let manualUrl = $state("");
  let scan = $state<
    { port: number; kind: AiProviderKind; name: string; state: string; latencyMs: number | null; models: string[] | null }[] | null
  >(null);
  let scanning = $state(false);
  let status = $state("");
  let statusKind = $state<"" | "ok" | "err">("");
  let editing = $state<string | null>(null);
  let pickerOpen = $state(false);
  let keyField = $state<HTMLInputElement | null>(null);
  let editKeyField = $state<HTMLInputElement | null>(null);
  let editUrlField = $state<HTMLInputElement | null>(null);
  let debounce: ReturnType<typeof setTimeout> | null = null;

  const local = $derived(!isCloudAiProviderKind(kind));
  const compat = $derived(kind === "openai_compatible");
  // ChatGPT is the one cloud kind with no key field: it connects via an OAuth
  // device-code login (the shared ChatgptConnect component), and the token set
  // lands in the vault slot a key would occupy.
  const oauth = $derived(kind === "chatgpt");
  // Set by the "Sign in with ChatGPT" click that also creates the instance, so
  // the freshly-mounted connect component starts the login without a second
  // click. Never armed on remount of an existing (still unconnected) instance.
  let chatgptAutostart = $state(false);
  const providers = $derived(ai.draftAiProviders);
  const chatgptInstance = $derived(providers.find((p) => p.kind === "chatgpt") ?? null);
  const rack = $derived(providers.length > 1);
  // The card owning the default model is pinned to the top of the rack.
  const racked = $derived(
    rack
      ? [...providers].sort(
          (a, b) =>
            (ai.draftAiDefaultModel?.provider === a.id ? 0 : 1) -
            (ai.draftAiDefaultModel?.provider === b.id ? 0 : 1),
        )
      : providers,
  );
  const liveProviders = $derived(
    providers.filter((p) => ai.aiVerifications[p.id]?.status === "live"),
  );
  const liveModelCount = $derived(
    liveProviders.reduce((total, p) => {
      const verification = ai.aiVerifications[p.id];
      return total + (verification?.status === "live" ? verification.models.length : 0);
    }, 0),
  );
  const scanFound = $derived((scan ?? []).filter((row) => row.models !== null));

  function verificationOf(id: string): AiVerification | undefined {
    return ai.aiVerifications[id];
  }
  function modelsOf(id: string): string[] {
    const verification = verificationOf(id);
    return verification?.status === "live" ? verification.models : [];
  }
  /** Why a provider's group in the readout is empty. */
  function emptyPoolReason(id: string): string {
    const verification = verificationOf(id);
    return verification?.status === "error" ? verification.reason : "no models listed yet";
  }
  function isDefault(providerId: string, model?: string): boolean {
    const ref = ai.draftAiDefaultModel;
    if (!ref || ref.provider !== providerId) return false;
    return model === undefined || ref.model === model;
  }
  function endpointOf(provider: AiProviderConfig): string {
    return provider.baseUrl.trim() || AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind] || "";
  }

  // ── The data-path line: one persistent line, not a fork ───────────────────
  const flowHost = $derived(
    local
      ? baseUrlHost(AI_LOCAL_DEFAULT_ENDPOINTS[kind] ?? "")
      : kind === "anthropic"
        ? "api.anthropic.com"
        : kind === "openai"
          ? "api.openai.com"
          : kind === "chatgpt"
            ? "chatgpt.com"
            : baseUrlHost(compatUrl) || "the host you type",
  );

  // ── Connecting ────────────────────────────────────────────────────────────
  function setStatus(text: string, tone: "" | "ok" | "err" = ""): void {
    status = text;
    statusKind = tone;
  }

  /** Read the typed key ONCE, hand it over, clear the field. Never stored. */
  async function connectCloud(): Promise<void> {
    const typed = keyField?.value ?? "";
    if (typed.trim().length < 6) {
      setStatus("");
      return;
    }
    if (compat && compatUrl.trim().length === 0) {
      setStatus("Add the base URL first — there is nowhere to send the key.", "err");
      return;
    }
    const id = ai.addProvider(kind, compat ? compatUrl.trim() : "");
    if (!id) return;
    if (keyField) keyField.value = "";
    setStatus(`reaching ${flowHost}…`);
    await ai.saveKeyAndVerify(id, typed);
    reportVerification(id);
  }

  async function connectLocal(target: AiProviderKind, baseUrl: string): Promise<void> {
    const id = ai.addProvider(target, baseUrl);
    if (!id) return;
    setStatus(`reaching ${baseUrlHost(baseUrl)}…`);
    await ai.verifyProvider(id);
    reportVerification(id);
  }

  function reportVerification(id: string): void {
    const verification = verificationOf(id);
    if (verification?.status === "live") {
      setStatus(
        `${plural(verification.models.length, "model")} listed in ${verification.latencyMs} ms`,
        "ok",
      );
      // First engine to answer seeds the default model, so the common case
      // needs no second decision.
      if (!ai.draftAiDefaultModel && verification.models.length > 0) {
        ai.draftAiDefaultModel = { provider: id, model: verification.models[0] };
      }
    } else if (verification?.status === "error") {
      setStatus(verification.reason, "err");
    } else {
      setStatus("");
    }
  }

  // The `chatgpt_login_update` outcome is owned by the onboarding STORE, not
  // this panel — see `subscribeChatgptLoginUpdates`. This panel is mounted
  // inside ChangeSettingsScreen's tab fork, so it is unmounted by any tab click
  // during the 15-minute approval wait, which is exactly when the user is away
  // in the browser. A listener here would drop the one terminal event.

  /** A ChatGPT sign-in or disconnect landed — re-prove the instance. The
   *  store re-probes vault presence first: the login writes the token set
   *  outside the key-input flow, so nothing else refreshes the "✓ signed in"
   *  pill or the connect button's Reconnect/Disconnect affordances. */
  async function onChatgptChanged(id: string): Promise<void> {
    setStatus("checking the ChatGPT connection…");
    await ai.handleChatgptChanged(id);
    reportVerification(id);
  }

  function onKeyInput(): void {
    if (debounce) clearTimeout(debounce);
    setStatus("waiting for you to finish typing…");
    debounce = setTimeout(() => void connectCloud(), 650);
  }

  async function runScan(): Promise<void> {
    scanning = true;
    scan = SCAN_TARGETS.map((target) => ({
      ...target,
      state: "trying",
      latencyMs: null,
      models: null,
    }));
    // In parallel: an unanswered port refuses immediately, and a black-holed one
    // is bounded by the command's own request timeout.
    await Promise.all(
      SCAN_TARGETS.map(async (target, index) => {
        const result = await ai.probeEndpoint(target.kind, `http://localhost:${target.port}`);
        const rows = scan;
        if (!rows) return;
        rows[index] = {
          ...rows[index],
          state: result ? "live" : "dead",
          latencyMs: result?.latencyMs ?? null,
          models: result?.models ?? null,
        };
        scan = [...rows];
      }),
    );
    scanning = false;
  }

  function alreadyConnected(port: number): boolean {
    return providers.some((p) => baseUrlHost(endpointOf(p)) === `localhost:${port}`);
  }

  async function connectManual(): Promise<void> {
    const typed = manualUrl.trim();
    if (!typed) return;
    manualUrl = "";
    await connectLocal(kind, typed.includes("://") ? typed : `http://${typed}`);
  }

  /** Save an edited key / endpoint on an existing card, then re-verify it. */
  async function saveEdit(provider: AiProviderConfig): Promise<void> {
    const typedKey = editKeyField?.value ?? "";
    const typedUrl = editUrlField?.value;
    if (typedUrl !== undefined) provider.baseUrl = typedUrl.trim();
    if (editKeyField) editKeyField.value = "";
    editing = null;
    // The config changed, so the old proof is void until this returns.
    ai.invalidateVerification(provider.id);
    setStatus(`reaching ${baseUrlHost(endpointOf(provider)) || flowHost}…`);
    if (typedKey.trim().length > 0) {
      await ai.saveKeyAndVerify(provider.id, typedKey);
    } else {
      await ai.verifyProvider(provider.id);
    }
    reportVerification(provider.id);
  }

  function pick(providerId: string | null, model: string | null): void {
    ai.draftAiDefaultModel = providerId && model ? { provider: providerId, model } : null;
    pickerOpen = false;
  }

  function chooseLater(): void {
    mode = "later";
    if (aiEnabled) onToggleAi(false);
  }
</script>

<div class="ob-ai">
  <div class="ob-head">
    <span class="mx-kicker">{mode === "later" ? "AI features · off" : "AI features · optional"}</span>
    <p class="ob-fine">
      Ask AI, daily digests and User Context need a reasoning engine. Recording, search,
      transcription and speaker names do not — they work either way.
    </p>
  </div>

  {@render notices?.()}

  <!-- ── The fork: two tiles; skipping is a first-class choice ────────────── -->
  <div class="mx-choice" role="radiogroup" aria-label="AI features">
    <label class="mx-choice__opt">
      <input type="radio" name="ob-ai-mode" value="later" checked={mode === "later"} onchange={chooseLater} />
      <span class="mx-choice__title">Set this up later</span>
      <span class="mx-choice__desc">Skipping downloads nothing and needs no account.</span>
    </label>
    <label class="mx-choice__opt">
      <input type="radio" name="ob-ai-mode" value="now" checked={mode === "now"} onchange={() => (mode = "now")} />
      <span class="mx-choice__title">I have a key — set it up now</span>
      <span class="mx-choice__desc">A local engine on this Mac, or your own cloud account.</span>
    </label>
  </div>

  {#if mode === "later"}
    <!-- ── Skipped: exactly what stays dark, and where to turn it on ──────── -->
    <div class="ob-ai-row">
      <div>
        <div class="t">These three stay dark</div>
        <div class="d">
          <b>Ask AI</b> (no questions about your day), <b>daily digests</b> (nothing is written each
          evening) and <b>User Context</b> (nothing is distilled about you).
        </div>
      </div>
      <input type="checkbox" role="switch" class="mx-switch" checked={false} aria-label="AI features"
        onchange={() => (mode = "now")} />
    </div>
    <p class="ob-fine">
      These keep working: recording, timeline, search, transcription, speaker names, privacy exclusions.
    </p>
    <p class="ob-fine">
      Turn it on later at <b>Settings → Intelligence → AI features</b>. Nothing needs re-running.
    </p>
  {:else if mode === "now"}
    <!-- ── Connecting ─────────────────────────────────────────────────────── -->
    <div class="ob-ai-row">
      <div>
        <div class="t">AI features</div>
        <div class="d">
          {ai.aiConfigReady && ai.draftAiDefaultModel
            ? `Ready — ${ai.draftAiDefaultModel.model} will answer.`
            : ai.aiConfigMissing}
        </div>
      </div>
      <input type="checkbox" role="switch" class="mx-switch" checked={aiEnabled} aria-label="AI features"
        onchange={(e) => onToggleAi(e.currentTarget.checked)} />
    </div>

    <p class="ob-fine">
      {local
        ? "If one is already running here, nothing has to be typed."
        : oauth
          ? "Your own ChatGPT Plus/Pro plan. Sign in in the browser — no API key."
          : "Your own account. There is no Save — the key proves itself."}
    </p>

    <div class="ob-acts">
      <select
        class="ob-select"
        aria-label="Provider"
        bind:value={kind}
        onchange={() => {
          scan = null;
          setStatus("");
        }}
      >
        <optgroup label="On this Mac — nothing leaves">
          {#each LOCAL_KINDS as k (k)}<option value={k}>{aiProviderKindLabel(k)}</option>{/each}
        </optgroup>
        <optgroup label="Cloud — your words leave this Mac">
          {#each CLOUD_KINDS as k (k)}<option value={k}>{aiProviderKindLabel(k)}</option>{/each}
        </optgroup>
      </select>
      {#if local}
        <button class="mx-btn" type="button" disabled={scanning} onclick={() => void runScan()}>
          {scanning ? "Scanning…" : scan ? "Scan again" : "Scan this Mac"}
        </button>
      {:else if oauth}
        {#if !chatgptInstance}
          <button
            class="mx-btn"
            type="button"
            onclick={async () => {
              chatgptAutostart = true;
              ai.addProvider("chatgpt");
              // One-shot: the connect component reads the flag once, on mount.
              // Left armed, every later remount starts ANOTHER device login.
              await tick();
              chatgptAutostart = false;
            }}
          >
            Sign in with ChatGPT
          </button>
        {/if}
      {:else}
        <!-- No `value` binding: the string is read from this node once and cleared. -->
        <input
          bind:this={keyField}
          class="mx-input ob-key"
          type="password"
          aria-label="API key"
          aria-invalid={statusKind === "err" || undefined}
          autocomplete="off"
          spellcheck="false"
          data-1p-ignore
          data-lpignore="true"
          placeholder="paste your key — it verifies itself"
          oninput={onKeyInput}
        />
        <button
          class="mx-btn"
          type="button"
          onclick={() => {
            if (debounce) clearTimeout(debounce);
            void connectCloud();
          }}
        >
          Verify now
        </button>
      {/if}
    </div>

    {#if compat}
      <input
        class="mx-input ob-key"
        aria-label="Base URL"
        autocomplete="off"
        spellcheck="false"
        placeholder="https://api.example.com/v1"
        bind:value={compatUrl}
      />
    {/if}

    <!-- Not while the same instance's CARD is open below: two mounted connect
         components for one provider means two live device codes. -->
    {#if oauth && chatgptInstance && editing !== chatgptInstance.id}
      <ChatgptConnect
        providerId={chatgptInstance.id}
        connected={!!ai.aiProviderKeySaved[chatgptInstance.id]}
        autostart={chatgptAutostart}
        onchange={() => chatgptInstance && void onChatgptChanged(chatgptInstance.id)}
      />
    {/if}

    {#if status}
      <span class="mx-inline" data-tone={statusKind === "ok" ? "ok" : statusKind === "err" ? "danger" : undefined} role="status">
        {#if statusKind === "ok"}<IconCheck width="13" height="13" />{:else if statusKind === "err"}<IconAlert
            width="13"
            height="13"
          />{:else}<span class="mx-spin mx-spin--sm"></span>{/if}{status}
      </span>
    {/if}

    <div class="ob-wire" class:out={!local}>
      <span>this Mac</span><i></i><span>{flowHost}</span>
      <span>{local ? "· nothing leaves" : "· prompts and their context leave"}</span>
    </div>

    {#if local && scan}
      <div class="ob-scan">
        {#each scan as row (row.port)}
          <div class:dead={row.state === "dead"}>
            <span class="ob-lamp" class:live={row.state === "live"}></span>
            <span>localhost:{row.port} · {row.name}</span>
            <span>{#if row.models}<span class="mx-chip" class:mx-chip--accent={row.models.length > 0}
                  class:mx-chip--warn={row.models.length === 0}>{plural(row.models.length, "model")}</span
                >{/if}</span>
            <span>
              {row.latencyMs != null ? `${row.latencyMs} ms` : row.state === "trying" ? "…" : "no answer"}
            </span>
            <span>
              {#if row.models}
                {#if alreadyConnected(row.port)}
                  <span class="mx-chip">connected</span>
                {:else}
                  <button
                    class="mx-btn mx-btn--sm"
                    type="button"
                    onclick={() => void connectLocal(row.kind, `http://localhost:${row.port}`)}
                  >
                    Use this
                  </button>
                {/if}
              {/if}
            </span>
          </div>
        {/each}
      </div>
      {#if !scanning && scanFound.length === 0}
        <div class="mx-empty mx-empty--compact" data-tone="warn">
          <span class="mx-empty__glyph"><IconSearch width="14" height="14" /></span>
          <b class="mx-empty__title">Nothing is listening on the usual ports</b>
          <p class="mx-empty__text">
            That is an answer, not an error. Start Ollama and scan again, point at an engine on your
            network below, or pick a cloud account above.
          </p>
        </div>
      {/if}
    {/if}

    {#if local}
      <div class="ob-acts">
        <input
          class="mx-input ob-key"
          aria-label="Endpoint on another machine"
          autocomplete="off"
          spellcheck="false"
          placeholder="or an engine on your network — studio.local:11434"
          bind:value={manualUrl}
        />
        <button class="mx-btn" type="button" onclick={() => void connectManual()}>Connect</button>
      </div>
    {/if}

    <!-- ── The connections. From the second on, they rack. ─────────────────── -->
    {#if rack}
      <p class="ob-fine">
        {plural(providers.length, "connection")} — each one is its own instance with its own key, its own
        endpoint and its own model list.
      </p>
    {/if}
    {#each racked as provider (provider.id)}
      {@const v = verificationOf(provider.id)}
      <div class="mx-panel ob-prov">
        <div class="ob-prov__head">
          <span class="ob-lamp" class:live={v?.status === "live"} class:bad={v?.status === "error"}></span>
          <span class="name">{ai.aiProviderInstanceLabel(provider)}</span>
          {#if isDefault(provider.id)}<span class="mx-chip mx-chip--info">default</span>{/if}
          <span class="mx-chip" class:mx-chip--accent={v?.status === "live"} class:mx-chip--danger={v?.status === "error"}>
            {aiVerificationWord(v)}
          </span>
          {#if isCloudAiProviderKind(provider.kind) && ai.aiProviderKeySaved[provider.id]}
            <span class="mx-chip" title="stored in the app vault, day.mnema.vault">
              <IconCheck width="11" height="11" />{provider.kind === "chatgpt" ? "signed in" : "key in vault"}
            </span>
          {/if}
        </div>

        {#if modelsOf(provider.id).length > 0}
          <div class="ob-acts">
            {#each modelsOf(provider.id) as model (model)}
              <button
                class="mx-btn mx-btn--sm"
                type="button"
                aria-pressed={isDefault(provider.id, model)}
                onclick={() => pick(provider.id, model)}
              >
                {model}
              </button>
            {/each}
          </div>
        {/if}

        {#if editing === provider.id}
          {#if provider.kind === "chatgpt"}
            <!-- Editing a ChatGPT card is a re-login, not a key replacement. -->
            <ChatgptConnect
              providerId={provider.id}
              connected={!!ai.aiProviderKeySaved[provider.id]}
              onchange={() => void onChatgptChanged(provider.id)}
            />
            <div class="ob-acts">
              <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={() => (editing = null)}>
                Done
              </button>
            </div>
          {:else}
            <div class="ob-acts">
              {#if isCloudAiProviderKind(provider.kind)}
                <input
                  bind:this={editKeyField}
                  class="mx-input ob-key"
                  type="password"
                  aria-label="Replacement API key"
                  autocomplete="off"
                  spellcheck="false"
                  data-1p-ignore
                  data-lpignore="true"
                  placeholder="a key is stored — type a new one to replace it"
                />
              {:else}
                <input
                  bind:this={editUrlField}
                  class="mx-input ob-key"
                  aria-label="Endpoint"
                  autocomplete="off"
                  spellcheck="false"
                  value={provider.baseUrl}
                  placeholder={AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind] ?? "http://localhost"}
                />
              {/if}
              <button class="mx-btn mx-btn--sm" type="button" onclick={() => void saveEdit(provider)}>
                Connect &amp; verify
              </button>
              <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={() => (editing = null)}>
                Cancel
              </button>
            </div>
          {/if}
        {:else}
          <div class="ob-acts">
            <button
              class="mx-btn mx-btn--sm"
              type="button"
              onclick={async () => {
                await ai.verifyProvider(provider.id);
                reportVerification(provider.id);
              }}
            >
              Test
            </button>
            <button class="mx-btn mx-btn--sm" type="button" onclick={() => (editing = provider.id)}>Edit</button>
            <button
              class="mx-btn mx-btn--ghost mx-btn--sm"
              type="button"
              disabled={ai.aiProviderRemoving}
              onclick={() => void ai.removeProvider(provider.id)}
            >
              Remove
            </button>
          </div>
        {/if}
      </div>
    {/each}

    <!-- ── The readout. Disabled until a provider exists. ──────────────────── -->
    <div class="ob-ai-row">
      <div>
        <div class="t">Default model</div>
        <div class="d">
          {providers.length === 0
            ? "Nothing to choose from yet — connect an engine first."
            : "Only models a connected engine listed just now."}
        </div>
      </div>
      <div class="mx-pop-anchor">
        <button
          class="mx-btn"
          type="button"
          aria-haspopup="listbox"
          aria-expanded={pickerOpen}
          disabled={providers.length === 0}
          onclick={() => (pickerOpen = !pickerOpen)}
        >
          {ai.draftAiDefaultModel?.model ?? "Choose a default model"}<IconChevron width="13" height="13" />
        </button>
        <div class="mx-pop" data-open={pickerOpen || undefined} role="listbox" aria-label="Default model">
          <ul class="mx-menu">
            <li role="option" aria-selected={ai.draftAiDefaultModel === null} onclick={() => pick(null, null)}
              onkeydown={(e) => e.key === "Enter" && pick(null, null)} tabindex="0">
              No default model<span class="mx-menu__meta">AI stays off</span>
            </li>
            {#each providers as provider (provider.id)}
              <li class="grp" aria-hidden="true">
                {ai.aiProviderInstanceLabel(provider)}<span class="mx-menu__meta">{aiVerificationWord(verificationOf(provider.id))}</span>
              </li>
              {#each modelsOf(provider.id) as model (model)}
                <li role="option" aria-selected={isDefault(provider.id, model)} tabindex="0"
                  onclick={() => pick(provider.id, model)}
                  onkeydown={(e) => e.key === "Enter" && pick(provider.id, model)}>
                  {model}
                </li>
              {:else}
                <li class="grp empty" aria-hidden="true">{emptyPoolReason(provider.id)}</li>
              {/each}
            {/each}
          </ul>
        </div>
      </div>
    </div>

    <!-- Only where a key was actually typed. ChatGPT signs in in the browser. -->
    {#if providers.some((p) => isCloudAiProviderKind(p.kind) && p.kind !== "chatgpt")}
      <p class="ob-fine">{VAULT_NOTE}</p>
    {/if}
    {#if chatgptInstance}
      <p class="ob-fine">
        The ChatGPT sign-in token is stored in the app's encrypted vault (day.mnema.vault, unlocked by
        one keychain item) — never in a config file. Usage counts against your own plan.
      </p>
    {/if}
    {#if liveProviders.length > 0}
      <p class="ob-fine">
        {plural(liveModelCount, "model")} across {plural(liveProviders.length, "verified engine")}.
      </p>
    {/if}
  {/if}
</div>

<style>
  .ob-ai {
    display: grid;
    gap: var(--s-4);
    justify-items: stretch;
  }
  .ob-ai > :global(.mx-inline) {
    justify-self: start;
  }
  .ob-ai-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-4);
    padding: 10px 0;
    border-top: 1px solid var(--mx-hairline);
    border-bottom: 1px solid var(--mx-hairline);
  }
  .t {
    font: 500 var(--text-md) / 1.25 var(--font-sans);
    color: var(--app-text-strong);
  }
  .d {
    margin-top: 2px;
    font: 400 var(--text-base) / 1.35 var(--font-sans);
    color: var(--app-text-subtle);
    text-wrap: pretty;
  }
  .d b,
  .ob-fine b {
    font-weight: 500;
    color: var(--app-text);
  }
  .ob-select {
    appearance: none;
    height: var(--h-md);
    min-width: 220px;
    padding: 0 30px 0 var(--s-3);
    cursor: pointer;
    border-radius: var(--r-md);
    border: 1px solid var(--app-border-strong);
    color: var(--app-text-strong);
    font: 400 var(--text-md) / 1 var(--font-sans);
    background: var(--app-surface-raised) no-repeat;
    background-image:
      linear-gradient(45deg, transparent 50%, var(--app-text-subtle) 50%),
      linear-gradient(-45deg, transparent 50%, var(--app-text-subtle) 50%);
    background-size: 5px 5px, 5px 5px;
    background-position: right 15px center, right 10px center;
  }
  .ob-select:focus-visible {
    outline: 2px solid var(--app-accent);
    outline-offset: 1px;
  }
  .ob-key {
    width: 300px;
    max-width: 100%;
    font-family: var(--font-mono);
    font-size: var(--text-base);
  }
  .ob-wire {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font: 400 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .ob-wire i {
    width: 64px;
    height: 1px;
    background: repeating-linear-gradient(90deg, var(--app-border-hover) 0 4px, transparent 4px 7px);
  }
  .ob-wire.out {
    color: var(--app-warn);
  }
  .ob-scan {
    display: grid;
    border: 1px solid var(--app-border);
    border-radius: var(--r-md);
    background: var(--app-surface);
  }
  .ob-scan > div {
    display: grid;
    grid-template-columns: 8px minmax(0, 1fr) auto 80px 96px;
    align-items: center;
    gap: var(--s-3);
    height: 40px;
    padding: 0 var(--s-3);
    font: 400 var(--text-base) / 1 var(--font-mono);
    color: var(--app-text);
  }
  .ob-scan > div + div {
    border-top: 1px solid var(--mx-hairline);
  }
  .ob-scan > div.dead {
    color: var(--app-text-subtle);
  }
  .ob-scan > div > span:last-child {
    justify-self: end;
  }
  .ob-lamp {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--chart-grey-2);
  }
  .ob-lamp.live {
    background: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
  .ob-lamp.bad {
    background: var(--app-danger);
  }
  .ob-prov {
    display: grid;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-4);
  }
  .ob-prov__head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  .ob-prov__head .name {
    font: 500 var(--text-md) / 1.2 var(--font-sans);
    color: var(--app-text-strong);
    margin-right: var(--s-2);
  }
  .mx-pop {
    min-width: 280px;
    max-height: 320px;
    overflow: auto;
  }
  .mx-menu li[aria-selected="true"] {
    background: var(--mx-selected);
    color: var(--app-text-strong);
  }
  .mx-menu li.grp {
    cursor: default;
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    background: none;
  }
  .mx-menu li.grp.empty {
    text-transform: none;
    letter-spacing: 0;
  }
</style>
