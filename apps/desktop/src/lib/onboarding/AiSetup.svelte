<!--
  Onboarding AI setup (issue #195, slice 11).

  Ported from `docs/onboarding/mockups/input-components/parts/aisetup.part.html`
  — that mockup is the design of record; behaviour and copy come from it.

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

  Motion: none ambient. The only animation is model chips arriving, which is real
  progress, and it is dropped under prefers-reduced-motion.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import ChatgptConnect from "$lib/components/ChatgptConnect.svelte";
  import Switch from "$lib/components/Switch.svelte";
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
  }: { ai: AiSetupStore; aiEnabled: boolean; onToggleAi: (on: boolean) => void } = $props();

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
  function lampClass(id: string): string {
    switch (verificationOf(id)?.status) {
      case "live": return "live";
      case "error": return "bad";
      case "checking": return "busy";
      default: return "";
    }
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
        `✓ ${plural(verification.models.length, "model")} listed in ${verification.latencyMs} ms`,
        "ok",
      );
      // First engine to answer seeds the default model, so the common case
      // needs no second decision.
      if (!ai.draftAiDefaultModel && verification.models.length > 0) {
        ai.draftAiDefaultModel = { provider: id, model: verification.models[0] };
      }
    } else if (verification?.status === "error") {
      setStatus(`✗ ${verification.reason}`, "err");
    } else {
      setStatus("");
    }
  }

  /** A ChatGPT sign-in or disconnect landed — re-prove the instance. */
  async function onChatgptChanged(id: string): Promise<void> {
    ai.invalidateVerification(id);
    setStatus("checking the ChatGPT connection…");
    await ai.verifyProvider(id);
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

{#if mode === null}
  <!-- ── The fork: skipping is the primary path ─────────────────────────── -->
  <span class="ob-m">AI features · optional</span>
  <p class="ob-fine lead">
    Ask AI, daily digests and User Context need a reasoning engine. Recording, search,
    transcription and speaker names do not — they work either way.
  </p>
  <div class="choose">
    <button class="ob-btn primary loud" type="button" onclick={chooseLater}>
      Set this up later
    </button>
    <button class="ob-btn loud" type="button" onclick={() => (mode = "now")}>
      I have a key — set it up now
    </button>
  </div>
  <div class="loss">
    <span>Skipping downloads nothing and needs no account.</span>
    <span>Everything else on this screen keeps working exactly the same.</span>
    <span>One switch in Settings → Intelligence turns it on whenever you want it.</span>
  </div>
{:else if mode === "later"}
  <!-- ── Skipped: exactly what stays dark, and where to turn it on ──────── -->
  <span class="ob-m">AI features · off</span>
  <div class="row first">
    <div class="grow">
      <div class="t">These three stay dark</div>
      <div class="d">
        <b>Ask AI</b> (no questions about your day), <b>daily digests</b> (nothing is written
        each evening) and <b>User Context</b> (nothing is distilled about you).
      </div>
    </div>
    <Switch checked={false} ariaLabel="AI features" onCheckedChange={() => (mode = "now")} />
  </div>
  <div class="row">
    <div class="grow">
      <div class="t">These keep working</div>
      <div class="d">
        Recording, timeline, search, transcription, speaker names, privacy exclusions.
      </div>
    </div>
  </div>
  <p class="ob-fine">
    Turn it on later at <b>Settings → Intelligence → AI features</b>. Nothing needs re-running.
  </p>
  <div class="ob-acts">
    <button class="ob-btn sm" type="button" onclick={() => (mode = "now")}>
      Actually, set it up now
    </button>
  </div>
{:else}
  <!-- ── Connecting ─────────────────────────────────────────────────────── -->
  <span class="ob-m">AI features</span>

  <div class="row first">
    <div class="grow">
      <div class="t">AI features</div>
      <div class="d">
        {ai.aiConfigReady && ai.draftAiDefaultModel
          ? `Ready — ${ai.draftAiDefaultModel.model} will answer.`
          : ai.aiConfigMissing}
      </div>
    </div>
    <Switch checked={aiEnabled} ariaLabel="AI features" onCheckedChange={onToggleAi} />
  </div>

  <div class="row block">
    <div class="t">{providers.length ? "Connect another engine" : "Connect an engine"}</div>
    <div class="d gap">
      {local
        ? "If one is already running here, nothing has to be typed."
        : oauth
          ? "Your own ChatGPT Plus/Pro plan. Sign in in the browser — no API key."
          : "Your own account. There is no Save — the key proves itself."}
    </div>

    <div class="line">
      <select
        class="fld"
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
        <button class="ob-btn sm" type="button" disabled={scanning} onclick={() => void runScan()}>
          {scanning ? "Scanning…" : scan ? "Scan again" : "Scan this Mac"}
        </button>
      {:else if oauth}
        {#if !chatgptInstance}
          <button
            class="ob-btn sm"
            type="button"
            onclick={() => {
              chatgptAutostart = true;
              ai.addProvider("chatgpt");
            }}
          >
            Sign in with ChatGPT
          </button>
        {/if}
      {:else}
        <!-- No `value` binding: the string is read from this node once and cleared. -->
        <input
          bind:this={keyField}
          class="fld"
          type="password"
          aria-label="API key"
          autocomplete="off"
          spellcheck="false"
          data-1p-ignore
          data-lpignore="true"
          placeholder="paste your key — it verifies itself"
          oninput={onKeyInput}
        />
      {/if}
    </div>

    {#if compat}
      <input
        class="fld top"
        aria-label="Base URL"
        autocomplete="off"
        spellcheck="false"
        placeholder="https://api.example.com/v1"
        bind:value={compatUrl}
      />
    {/if}

    {#if oauth && chatgptInstance}
      <div class="top">
        <ChatgptConnect
          providerId={chatgptInstance.id}
          connected={!!ai.aiProviderKeySaved[chatgptInstance.id]}
          autostart={chatgptAutostart}
          onchange={() => chatgptInstance && void onChatgptChanged(chatgptInstance.id)}
        />
      </div>
    {/if}

    <div class="flow" class:out={!local}>
      <span>this Mac</span>
      <span class="wire"></span>
      <span>{flowHost}</span>
      <span>{local ? "· nothing leaves" : "· prompts and their context leave"}</span>
    </div>

    {#if local && scan}
      <div class="scan">
        {#each scan as row (row.port)}
          <div class="sr">
            <span
              class="lamp"
              class:live={row.state === "live"}
              class:busy={row.state === "trying"}
              class:bad={row.state === "dead"}
            ></span>
            <span class="addr">localhost:{row.port} · {row.name}</span>
            {#if row.models}<span class="pill ok">{plural(row.models.length, "model")}</span>{/if}
            <span class="ms">
              {row.latencyMs != null
                ? `${row.latencyMs} ms`
                : row.state === "trying"
                  ? "…"
                  : "no answer"}
            </span>
            {#if row.models}
              {#if alreadyConnected(row.port)}
                <span class="pill">connected</span>
              {:else}
                <button
                  class="ob-btn sm"
                  type="button"
                  onclick={() => void connectLocal(row.kind, `http://localhost:${row.port}`)}
                >
                  Use this
                </button>
              {/if}
            {/if}
          </div>
        {/each}
      </div>
      {#if !scanning && scanFound.length === 0}
        <p class="ob-fine top">
          Nothing is listening on the usual ports — that is an answer, not an error. Start Ollama
          and scan again, point at an engine on your network below, or pick a cloud account above.
        </p>
      {/if}
    {/if}

    {#if local}
      <div class="line top">
        <input
          class="fld"
          aria-label="Endpoint on another machine"
          autocomplete="off"
          spellcheck="false"
          placeholder="or an engine on your network — studio.local:11434"
          bind:value={manualUrl}
        />
        <button class="ob-btn sm" type="button" onclick={() => void connectManual()}>
          Connect
        </button>
      </div>
    {:else if !oauth}
      <div class="ob-acts top">
        <button
          class="ob-btn sm"
          type="button"
          onclick={() => {
            if (debounce) clearTimeout(debounce);
            void connectCloud();
          }}
        >
          Verify now
        </button>
      </div>
    {/if}

    {#if status}
      <p class="ob-fine status" class:ok={statusKind === "ok"} class:err={statusKind === "err"}>
        {status}
      </p>
    {/if}
  </div>

  <!-- ── The connections. From the second on, they rack. ─────────────────── -->
  {#if rack}
    <p class="ob-fine top">
      {plural(providers.length, "connection")} — each one is its own instance with its own key,
      its own endpoint and its own model list.
    </p>
  {/if}
  {#each racked as provider (provider.id)}
    <div class="prov" class:pinned={rack && isDefault(provider.id)}>
      <div class="prov-head">
        <span class="lamp {lampClass(provider.id)}"></span>
        <span class="name">{ai.aiProviderInstanceLabel(provider)}</span>
        {#if isDefault(provider.id)}<span class="pill info">default</span>{/if}
        <span
          class="pill"
          class:ok={verificationOf(provider.id)?.status === "live"}
          class:bad={verificationOf(provider.id)?.status === "error"}
        >
          {aiVerificationWord(verificationOf(provider.id))}
        </span>
        {#if isCloudAiProviderKind(provider.kind) && ai.aiProviderKeySaved[provider.id]}
          <span class="pill" title="stored in the app vault, day.mnema.vault">
            {provider.kind === "chatgpt" ? "✓ signed in" : "✓ key in vault"}
          </span>
        {/if}
      </div>

      {#if modelsOf(provider.id).length > 0}
        <div class="models">
          {#each modelsOf(provider.id) as model (model)}
            <button
              class="mchip"
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
          <div class="top">
            <ChatgptConnect
              providerId={provider.id}
              connected={!!ai.aiProviderKeySaved[provider.id]}
              onchange={() => void onChatgptChanged(provider.id)}
            />
          </div>
          <div class="ob-acts top">
            <button class="ob-btn sm ghost" type="button" onclick={() => (editing = null)}>
              Done
            </button>
          </div>
        {:else}
        <div class="line top">
          {#if isCloudAiProviderKind(provider.kind)}
            <input
              bind:this={editKeyField}
              class="fld"
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
              class="fld"
              aria-label="Endpoint"
              autocomplete="off"
              spellcheck="false"
              value={provider.baseUrl}
              placeholder={AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind] ?? "http://localhost"}
            />
          {/if}
          <button class="ob-btn sm" type="button" onclick={() => void saveEdit(provider)}>
            Connect &amp; verify
          </button>
          <button class="ob-btn sm ghost" type="button" onclick={() => (editing = null)}>
            Cancel
          </button>
        </div>
        {/if}
      {:else}
        <div class="ob-acts top">
          <button
            class="ob-btn sm"
            type="button"
            onclick={async () => {
              await ai.verifyProvider(provider.id);
              reportVerification(provider.id);
            }}
          >
            Test
          </button>
          <button class="ob-btn sm" type="button" onclick={() => (editing = provider.id)}>
            Edit
          </button>
          <button
            class="ob-btn sm ghost"
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
  <div class="row">
    <div class="grow">
      <div class="t">Default model</div>
      <div class="d">
        {providers.length === 0
          ? "Nothing to choose from yet — connect an engine first."
          : "Only models a connected engine listed just now."}
      </div>
    </div>
    <details class="picker" bind:open={pickerOpen}>
      <summary
        aria-disabled={providers.length === 0}
        aria-haspopup="listbox"
        onclick={(event) => {
          if (providers.length === 0) event.preventDefault();
        }}
      >
        <span class:ph={!ai.draftAiDefaultModel}>
          {ai.draftAiDefaultModel?.model ?? "Choose a default model"}
        </span>
        <span aria-hidden="true">▾</span>
      </summary>
      <div class="pop" role="listbox" aria-label="Default model">
        <button
          class="opt"
          type="button"
          role="option"
          aria-selected={ai.draftAiDefaultModel === null}
          onclick={() => pick(null, null)}
        >
          <span>No default model</span><span class="side">AI stays off</span>
        </button>
        {#each providers as provider (provider.id)}
          <div class="grp">
            <span>{ai.aiProviderInstanceLabel(provider)}</span>
            <span>{aiVerificationWord(verificationOf(provider.id))}</span>
          </div>
          {#each modelsOf(provider.id) as model (model)}
            <button
              class="opt"
              type="button"
              role="option"
              aria-selected={isDefault(provider.id, model)}
              onclick={() => pick(provider.id, model)}
            >
              <span>{model}</span><span class="side"></span>
            </button>
          {:else}
            <div class="opt empty">{emptyPoolReason(provider.id)}</div>
          {/each}
        {/each}
      </div>
    </details>
  </div>

  <!-- `aiRestoredModelNote` is printed once, at screen level, so it survives all
       three modes (fork, "later", "now"). -->
  {#if providers.some((p) => isCloudAiProviderKind(p.kind))}
    <p class="ob-fine top">{VAULT_NOTE}</p>
  {/if}
  {#if liveProviders.length > 0}
    <p class="ob-fine top">
      {plural(liveModelCount, "model")} across {plural(liveProviders.length, "verified engine")}.
    </p>
  {/if}

  <div class="ob-acts top">
    <button class="ob-btn sm ghost" type="button" onclick={chooseLater}>
      Never mind — set this up later
    </button>
  </div>
{/if}

<style>
  .lead {
    margin: 7px 0 0;
  }
  .top {
    margin-top: 10px;
  }

  /* rows */
  .row {
    display: flex;
    align-items: flex-start;
    gap: 16px;
    justify-content: space-between;
    padding: 11px 0;
    border-top: 1px solid var(--app-border);
  }
  .row.first {
    border-top: 0;
  }
  .row.block {
    display: block;
  }
  .t {
    color: var(--app-text-strong);
    font-size: var(--text-md);
  }
  .d {
    color: var(--app-text-subtle);
    font-size: var(--text-sm);
    max-width: 48ch;
  }
  .d.gap {
    margin-bottom: 9px;
  }
  .d b {
    color: var(--app-text-strong);
    font-weight: 400;
  }
  .grow {
    flex: 1 1 auto;
    min-width: 0;
  }
  /* `Switch` is a Settings-row component: its wrapper is `width: 100%` because
     there it owns the label and the description too. Used here as a BARE toggle
     beside the row's own `.t`/`.d`, that 100% became the flex basis and ate the
     row — `.grow` was squeezed to 211px (description wrapped to two lines) and
     the 36px track was stranded a quarter of the way across, with 769px of
     nothing to its right. The mockup draws the switch flush with the row's
     right edge (`revision-2.html:3589`), which is what shrink-wrapping the cell
     restores. Same shape as the chain's `auto` control column in
     `FeatureSwitches`. */
  .row > :global(.switch-wrapper) {
    width: auto;
    flex: none;
  }

  /* fields */
  .fld {
    font: inherit;
    font-size: var(--text-sm);
    color: var(--app-text);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: 7px;
    height: var(--ob-ctl-h);
    padding: 0 12px;
    width: 100%;
    min-width: 0;
  }
  .fld::placeholder {
    color: var(--app-text-faint);
  }
  select.fld {
    cursor: pointer;
  }
  .line {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .line > .fld {
    flex: 1 1 200px;
  }
  .line > select.fld {
    flex: 0 1 200px;
  }
  /* A button standing on a field's row is the FIELD's height, not the inline
     one — that mismatch is the whole "assembled from parts" complaint. */
  .line > :global(.ob-btn) {
    min-height: var(--ob-ctl-h);
  }
  .fld:focus-visible,
  summary:focus-visible,
  .mchip:focus-visible,
  .opt:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  /* the fork */
  .choose {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    margin-top: 14px;
  }
  @media (max-width: 600px) {
    .choose {
      grid-template-columns: 1fr;
    }
  }
  .loud {
    padding: 17px 16px;
    font-size: var(--text-md);
    /* A whole-pixel line box, not a 1.4 ratio of a fractional font size —
       that pair put these two cards on 54.19px. 17 + 20 + 17 = 54. */
    line-height: 20px;
  }
  .loss {
    margin-top: 13px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .loss span {
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    display: flex;
    gap: 9px;
  }
  .loss span::before {
    content: "—";
    color: var(--app-text-faint);
  }

  /* data-path line — one persistent line, not a fork */
  .flow {
    margin-top: 10px;
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: var(--text-xs);
    color: var(--app-text-muted);
    border: 1px dashed var(--app-border-strong);
    border-radius: 8px;
    padding: 8px 11px;
  }
  .flow .wire {
    flex: 1 1 40px;
    height: 1px;
    background: repeating-linear-gradient(
      90deg,
      var(--app-text-faint) 0 5px,
      transparent 5px 10px
    );
  }
  .flow.out {
    color: var(--app-info);
    border-color: var(--app-info-border);
  }
  .flow.out .wire {
    background: repeating-linear-gradient(90deg, var(--app-info) 0 5px, transparent 5px 10px);
  }

  /* lamps + pills */
  .lamp {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: 0 0 auto;
    background: var(--app-text-faint);
    display: inline-block;
  }
  .lamp.live {
    background: var(--app-accent);
    box-shadow: 0 0 6px var(--app-accent-glow);
  }
  .lamp.bad {
    background: var(--app-danger);
  }
  .lamp.busy {
    background: var(--app-warn);
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    letter-spacing: 0.06em;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    padding: 3px 9px;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    text-transform: uppercase;
    white-space: nowrap;
  }
  .pill.ok {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  .pill.bad {
    color: var(--app-danger);
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
  }
  .pill.info {
    color: var(--app-info);
    border-color: var(--app-info-border);
    background: var(--app-info-bg);
  }

  /* the verdict line — no progress bar: a real probe has no progress stream,
     and an indeterminate one would be ambient motion. */
  .status {
    margin-top: 7px;
  }
  .status.ok {
    color: var(--app-accent);
  }
  .err {
    color: var(--app-danger);
  }

  /* scan ladder */
  .scan {
    border: 1px solid var(--app-border);
    border-radius: 9px;
    overflow: hidden;
    margin-top: 10px;
  }
  .sr {
    display: flex;
    align-items: center;
    gap: 10px;
    height: var(--ob-ctl-h);
    padding: 0 12px;
    font-size: var(--text-sm);
  }
  .sr + .sr {
    border-top: 1px solid var(--app-border);
  }
  .addr {
    color: var(--app-text-muted);
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ms {
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }

  /* connection card / rack */
  .prov {
    border: 1px solid var(--app-border);
    border-radius: 9px;
    padding: 11px 12px;
    margin-top: 8px;
    background: var(--app-surface-subtle);
  }
  .prov.pinned {
    border-color: var(--app-accent-border);
  }
  .prov-head {
    display: flex;
    align-items: center;
    gap: 9px;
    flex-wrap: wrap;
  }
  .prov-head .name {
    color: var(--app-text-strong);
    font-size: var(--text-sm);
    flex: 1 1 140px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .models {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    margin-top: 9px;
  }
  .mchip {
    font: inherit;
    font-size: var(--text-xs);
    border: 1px solid var(--app-border);
    background: var(--app-surface-raised);
    color: var(--app-text-muted);
    border-radius: 999px;
    padding: 4px 10px;
    cursor: pointer;
  }
  .mchip:hover {
    border-color: var(--app-border-hover);
  }
  .mchip[aria-pressed="true"] {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }

  /* default-model readout (native <details>) */
  .picker {
    position: relative;
    width: 290px;
    max-width: 100%;
    flex: 0 1 auto;
  }
  .picker > summary {
    list-style: none;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: var(--text-sm);
    border: 1px solid var(--app-border-strong);
    border-radius: 7px;
    height: var(--ob-ctl-h);
    padding: 0 12px;
    background: var(--app-surface-raised);
    color: var(--app-text);
  }
  .picker > summary::-webkit-details-marker {
    display: none;
  }
  .picker > summary[aria-disabled="true"] {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .picker > summary .ph {
    color: var(--app-text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pop {
    position: absolute;
    z-index: 10;
    right: 0;
    min-width: 100%;
    width: max-content;
    max-width: 360px;
    margin-top: 4px;
    max-height: 250px;
    overflow: auto;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    box-shadow: var(--app-shadow-popover);
    padding: 6px;
  }
  .grp {
    font-size: var(--text-xs);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    padding: 8px 8px 4px;
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }
  .opt {
    display: flex;
    width: 100%;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    text-align: left;
    font: inherit;
    font-size: var(--text-sm);
    color: var(--app-text);
    background: transparent;
    border: 0;
    border-radius: 6px;
    padding: 7px 8px;
    cursor: pointer;
  }
  .opt:hover {
    background: var(--app-surface-hover);
  }
  .opt.empty {
    color: var(--app-text-faint);
    cursor: default;
  }
  .opt[aria-selected="true"] {
    color: var(--app-accent);
  }
  .opt .side {
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
</style>
