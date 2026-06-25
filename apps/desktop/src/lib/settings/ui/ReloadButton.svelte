<script lang="ts">
  interface Props {
    onclick: () => void;
    /** Reload in flight — spins the icon and disables the button. */
    busy?: boolean;
    /** Disabled for a reason other than an in-flight reload (e.g. saving). */
    disabled?: boolean;
    /** Tooltip text + default accessible label (e.g. "Reload", "Refresh"). */
    title?: string;
    /** Accessible label override; falls back to `title`. */
    label?: string;
  }

  let { onclick, busy = false, disabled = false, title = "Reload", label }: Props = $props();
</script>

<button
  class="settings-icon-btn"
  class:settings-icon-btn--spin={busy}
  type="button"
  {title}
  aria-label={label ?? title}
  aria-busy={busy}
  disabled={disabled || busy}
  {onclick}
>
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
    <path d="M21 3v5h-5" />
    <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
    <path d="M3 21v-5h5" />
  </svg>
</button>
