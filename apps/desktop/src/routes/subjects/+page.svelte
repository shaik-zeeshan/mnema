<script lang="ts">
  // Subjects — a destination, not a surface (direction 01, mockup 09). Opened
  // from Overview's Subjects tile; `?s=<subject>` is the drill-in, so index and
  // detail are ONE route and the back chevron the shell layout draws already
  // reads `‹ Overview  Subjects` / `‹ Subjects  <name>` with no extra wiring.
  import { page } from "$app/stores";
  import SubjectsIndex from "$lib/subjects/SubjectsIndex.svelte";
  import SubjectPage from "$lib/subjects/SubjectPage.svelte";

  const subject = $derived($page.url.searchParams.get("s"));
</script>

{#if subject}
  {#key subject}
    <SubjectPage {subject} />
  {/key}
{:else}
  <SubjectsIndex />
{/if}
