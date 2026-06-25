<script lang="ts">
  import type { OnboardingController, PermissionKey } from "./onboarding.svelte";

  let { controller }: { controller: OnboardingController } = $props();

  // The three capture permissions, in the mockup's order. `key` is the backend
  // permission identifier; `title`/`sub` mirror the mockup copy verbatim.
  const rows: { key: PermissionKey; title: string; sub: string }[] = [
    {
      key: "screen",
      title: "Screen recording",
      sub: "Required to capture window video.",
    },
    {
      key: "microphone",
      title: "Microphone",
      sub: "Needed only if microphone capture is on.",
    },
    {
      key: "systemAudio",
      title: "System audio",
      sub: "macOS audio capture entitlement.",
    },
  ];

  // tone → pill modifier class (ok→granted, pending→pending, blocked→denied).
  const pillClass = (tone: "ok" | "pending" | "blocked"): string =>
    tone === "ok" ? "granted" : tone === "pending" ? "pending" : "denied";
</script>

<div class="group">
  {#each rows as row (row.key)}
    {@const value = controller.permissions?.[row.key]}
    {@const action = controller.permissionAction(value)}
    <div class="perm">
      <div class="pn">
        <div class="t">{row.title}</div>
        <div class="s">{row.sub}</div>
      </div>
      <div class="pr">
        <span class="pill {pillClass(controller.permissionTone(value))}">
          <span class="d"></span>{controller.permissionLabel(value)}
        </span>
        {#if action}
          <button
            type="button"
            class="btn sm"
            disabled={controller.requestingPerm === row.key}
            onclick={() => controller.requestPermission(row.key)}
          >
            {controller.requestingPerm === row.key ? "…" : action.label}
          </button>
        {/if}
      </div>
    </div>
  {/each}

  <div class="ctl">
    <div class="ctl-label">
      <div class="desc">Re-check after changing access in System Settings.</div>
    </div>
    <div class="ctl-field">
      <button
        type="button"
        class="btn sm"
        disabled={controller.refreshingPerms}
        onclick={() => controller.refreshPermissions()}
      >
        {controller.refreshingPerms ? "Checking…" : "Re-check"}
      </button>
    </div>
  </div>

  <!-- Optional Gecko (Firefox/Zen) browser-URL access. Shown only when a Gecko
       browser is installed; never gates progression. -->
  {#if controller.geckoInstalled}
    {@const trusted = controller.geckoTrusted}
    <div class="gecko-optional">
      <div class="group-title">Browser URLs · optional</div>
      <div class="perm">
        <div class="pn">
          <div class="t">
            {controller.geckoInstalledNames.length > 0
              ? `${controller.geckoInstalledNames.join(" / ")} page URLs`
              : "Firefox / Zen page URLs"}
          </div>
          <div class="s">
            Optional. Lets Mnema capture the page address for Firefox and Zen (they
            have no scriptable URL like Chrome/Safari). Requires the macOS
            Accessibility permission; enable Mnema under Privacy &amp; Security →
            Accessibility.
          </div>
        </div>
        <div class="pr">
          <span class="pill {trusted ? 'granted' : 'pending'}">
            <span class="d"></span>{trusted ? "Granted" : "Not granted"}
          </span>
          {#if !trusted}
            <button
              type="button"
              class="btn sm"
              disabled={controller.requestingGeckoAccess}
              onclick={() => controller.requestGeckoAccess()}
            >
              {controller.requestingGeckoAccess ? "Requesting…" : "Grant access"}
            </button>
          {/if}
        </div>
      </div>
      {#if !trusted}
        <div class="ctl">
          <div class="ctl-label">
            <div class="desc">Re-check after enabling Mnema in System Settings.</div>
          </div>
          <div class="ctl-field">
            <button type="button" class="btn sm" onclick={() => controller.openGeckoAccessSettings()}>
              Open Settings
            </button>
            <button
              type="button"
              class="btn sm"
              disabled={controller.recheckingGeckoAccess}
              onclick={() => controller.recheckGeckoAccess()}
            >
              {controller.recheckingGeckoAccess ? "Checking…" : "Recheck"}
            </button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* Optional browser-URL (Gecko) sub-section — visually separated from the
     required capture permissions above with a dashed divider. */
  .gecko-optional {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: 14px;
    margin-top: 2px;
    border-top: 1px dashed var(--app-border);
  }
</style>
