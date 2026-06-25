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
</div>
