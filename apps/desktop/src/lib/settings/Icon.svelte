<script lang="ts" module>
  import type { SettingsSectionId } from "./groups";

  // The shared settings-icon family — one glyph per settings sub-section
  // (mirrors `SettingsSectionId`) plus standalone glyphs (e.g. `lock`) used
  // by other surfaces such as onboarding. Every glyph is drawn on a 24 viewBox
  // with no intrinsic size/stroke: the consumer's CSS owns sizing/stroke/color
  // (in the rail, the global `.settings-shell .nav-item svg` selector).
  export type IconName = SettingsSectionId | "lock";
</script>

<script lang="ts">
  interface Props {
    name: IconName;
  }

  let { name }: Props = $props();
</script>

<svg viewBox="0 0 24 24" aria-hidden="true">
  {#if name === "appearance"}
    <path d="M12 2a10 10 0 1 0 0 20 2.5 2.5 0 0 0 2-4 2.5 2.5 0 0 1 2-4h2A4 4 0 0 0 22 10 10 10 0 0 0 12 2z" />
    <circle cx="7.5" cy="10.5" r="1" />
    <circle cx="12" cy="7.5" r="1" />
    <circle cx="16.5" cy="10.5" r="1" />
  {:else if name === "startup"}
    <path d="M12 2v8" />
    <path d="M18.4 6.6a9 9 0 1 1-12.8 0" />
  {:else if name === "shortcuts"}
    <rect x="2" y="6" width="20" height="12" rx="2" />
    <path d="M6 10h0M10 10h0M14 10h0M18 10h0M6 14h0M18 14h0M9 14h6" />
  {:else if name === "capture"}
    <rect x="2" y="3" width="20" height="14" rx="2" />
    <path d="M8 21h8M12 17v4" />
  {:else if name === "video"}
    <rect x="2" y="6" width="20" height="13" rx="2" />
    <path d="M2 9h20M6 6V4M10 6V4M14 6V4M18 6V4" />
  {:else if name === "audio"}
    <path d="M3 12h2l2-6 4 14 3-9 2 4h5" />
  {:else if name === "privacy"}
    <path d="M12 2l8 3v6c0 5-3.5 8-8 11-4.5-3-8-6-8-11V5l8-3z" />
  {:else if name === "intelligence"}
    <path d="M9 2v6M15 2v6M8 8h8v3a4 4 0 0 1-8 0V8zM12 15v3a3 3 0 0 0 3 3h2" />
  {:else if name === "askAi"}
    <path d="M12 3l1.8 4.2L18 9l-4.2 1.8L12 15l-1.8-4.2L6 9l4.2-1.8L12 3z" />
    <path d="M18 14l.9 2.1L21 17l-2.1.9L18 20l-.9-2.1L15 17l2.1-.9L18 14z" />
  {:else if name === "userContext"}
    <circle cx="12" cy="8" r="4" />
    <path d="M4 21a8 8 0 0 1 16 0" />
  {:else if name === "ocr"}
    <path d="M4 8V6a2 2 0 0 1 2-2h2M16 4h2a2 2 0 0 1 2 2v2M20 16v2a2 2 0 0 1-2 2h-2M8 20H6a2 2 0 0 1-2-2v-2" />
    <path d="M8 10h0M12 10v4M16 10h0M9 16h6" />
  {:else if name === "transcription"}
    <rect x="9" y="2" width="6" height="12" rx="3" />
    <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
  {:else if name === "speakers"}
    <circle cx="9" cy="8" r="3" />
    <path d="M3 20a6 6 0 0 1 12 0" />
    <path d="M16 5.5a3 3 0 0 1 0 5M18 14a6 6 0 0 1 3 5" />
  {:else if name === "semanticSearch"}
    <circle cx="11" cy="11" r="7" />
    <path d="M21 21l-4.3-4.3" />
    <circle cx="11" cy="11" r="1.5" fill="currentColor" stroke="none" />
  {:else if name === "storage"}
    <ellipse cx="12" cy="5" rx="8" ry="3" />
    <path d="M4 5v6c0 1.66 3.58 3 8 3s8-1.34 8-3V5" />
    <path d="M4 11v6c0 1.66 3.58 3 8 3s8-1.34 8-3v-6" />
  {:else if name === "access"}
    <circle cx="8" cy="8" r="4" />
    <path d="M11 11l8 8M16 16l2-2M18 18l2-2" />
  {:else if name === "about"}
    <circle cx="12" cy="12" r="9" />
    <path d="M12 16v-4M12 8h.01" />
  {:else if name === "developer"}
    <path d="M8 9l-4 3 4 3M16 9l4 3-4 3M13 6l-2 12" />
  {:else if name === "lock"}
    <rect x="5" y="11" width="14" height="9" rx="2" />
    <path d="M8 11V8a4 4 0 0 1 8 0v3" />
  {/if}
</svg>
