<script lang="ts">
  // Subjects destination (page 09) — tiers by conviction. Opened from the
  // Overview "Subjects" tile's door. The drill-in is addressable:
  // `/subjects?subject=⟨name⟩` renders the detail, and the shared title bar
  // reads "‹ Subjects · ⟨name⟩" for it (back pops the drill-in).
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import Subjects from "$lib/insights/Subjects.svelte";
  import SubjectDetail from "$lib/insights/SubjectDetail.svelte";

  const selectedSubject = $derived($page.url.searchParams.get("subject"));

  function openSubject(subject: string): void {
    void goto(`/subjects?subject=${encodeURIComponent(subject)}`);
  }

  function backToIndex(): void {
    void goto("/subjects");
  }
</script>

<div class="dest">
  <div class="dest__scroll">
    {#if selectedSubject}
      {#key selectedSubject}
        <SubjectDetail subject={selectedSubject} onBack={backToIndex} />
      {/key}
    {:else}
      <Subjects onOpenSubject={openSubject} />
    {/if}
  </div>
</div>

<style>
  .dest {
    flex: 1 1 auto;
    min-height: 0;
    margin-top: calc(var(--h-titlebar) * -1);
    position: relative;
    background: var(--app-bg);
    overflow: hidden;
  }
  /* The destination pane's own light: two soft corner washes UNDER the content,
     so the plates read as floating over a lit surface rather than on flat grey.
     Non-interactive, never behind text contrast (every plate is opaque). */
  .dest:before {
    content: "";
    position: absolute;
    inset: 0;
    pointer-events: none;
    background:
      radial-gradient(
        520px 320px at 4% 0%,
        color-mix(in srgb, var(--app-accent) 10%, transparent),
        transparent 70%
      ),
      radial-gradient(
        420px 380px at 98% 96%,
        color-mix(in srgb, var(--app-source-screen) 12%, transparent),
        transparent 72%
      );
  }
  .dest__scroll {
    position: absolute;
    inset: 0;
    overflow-y: auto;
    overflow-x: hidden;
    padding: calc(var(--h-titlebar) + 18px) var(--s-24) var(--s-24);
    scrollbar-width: thin;
    scrollbar-color: var(--app-border-hover) transparent;
  }
</style>
