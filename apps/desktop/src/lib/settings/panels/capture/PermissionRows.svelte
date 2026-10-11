<!--
  Settings → Capture: the macOS Screen Recording and Microphone permissions.
  Never asked → the macOS prompt; denied → System Settings (macOS won't
  re-prompt). Re-read on window focus, so a grant made in System Settings shows
  on return. System audio is not here — it has no authorization query and keeps
  its own inferred hint above (ADR 0052).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import { humanizeError } from "$lib/format-error";
  import { openPermissionHelper } from "$lib/shell/permission-helper";
  import type { GetPermissionsResponse, PermissionsMap } from "$lib/types";
  import IconLock from "~icons/lucide/lock";
  import IconExternal from "~icons/lucide/external-link";
  import IconAlert from "~icons/lucide/triangle-alert";
  import { PERMISSION_ROWS, permissionRowState, type PermissionKind } from "./permission-rows";

  let permissions = $state<PermissionsMap | null>(null);
  let busy = $state<PermissionKind | null>(null);
  let error = $state<{ kind: PermissionKind; text: string } | null>(null);

  // Non-fatal: a failed read hides the rows rather than accusing macOS.
  async function load() {
    try {
      permissions = (await invoke<GetPermissionsResponse>("get_capture_permissions")).permissions;
    } catch {
      permissions = null;
    }
  }
  onMount(load);

  async function run(kind: PermissionKind, action: () => Promise<void>) {
    if (busy) return;
    busy = kind;
    error = null;
    try {
      await action();
    } catch (err) {
      error = { kind, text: humanizeError(err) };
    } finally {
      busy = null;
    }
  }

  const request = (kind: PermissionKind) =>
    run(kind, async () => {
      permissions = (await invoke<GetPermissionsResponse>("request_capture_permission", { kind })).permissions;
    });

  // The one hook the permission helper window (ON3) replaces.
  const openSettings = (kind: PermissionKind) =>
    run(kind, () => openPermissionHelper(kind));
</script>

<svelte:window onfocus={load} />

{#if permissions}
  {#each PERMISSION_ROWS as row (row.kind)}
    {@const st = permissionRowState(permissions[row.kind])}
    {#if st === "denied"}
      <SettingRow label="{row.label} permission" description="macOS access Mnema needs to {row.what}." full>
        {#snippet control()}
          <div class="mx-notice" data-tone="warn">
            <span class="mx-notice__icon"><IconLock width="15" height="15" /></span>
            <b class="mx-notice__title">{row.label} · not allowed</b>
            <p class="mx-notice__text">
              Mnema can’t {row.what} until you allow it under Privacy &amp; Security → {row.pane}.
            </p>
            <div class="mx-notice__acts">
              <button type="button" class="mx-btn mx-btn--sm" onclick={() => openSettings(row.kind)} disabled={busy !== null}>
                Open System Settings<IconExternal width="12" height="12" />
              </button>
            </div>
          </div>
          {#if error?.kind === row.kind}
            <span class="mx-inline" data-tone="danger"><IconAlert width="13" height="13" />{error.text}</span>
          {/if}
        {/snippet}
      </SettingRow>
    {:else if st !== "hidden"}
      <SettingRow label="{row.label} permission" description="macOS access Mnema needs to {row.what}.">
        {#snippet control()}
          <div class="perm-ctl">
            {#if error?.kind === row.kind}
              <span class="mx-inline" data-tone="danger"><IconAlert width="13" height="13" />{error.text}</span>
            {/if}
            {#if st === "granted"}
              <span class="settings-state settings-state--ok">allowed</span>
            {:else}
              <button type="button" class="mx-btn" onclick={() => request(row.kind)} disabled={busy !== null} aria-busy={busy === row.kind}>
                {#if busy === row.kind}<span class="mx-spin mx-spin--sm"></span>{/if}Grant access
              </button>
            {/if}
          </div>
        {/snippet}
      </SettingRow>
    {/if}
  {/each}
{/if}

<style>
  .perm-ctl {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .mx-notice {
    width: 100%;
    box-sizing: border-box;
  }
</style>
