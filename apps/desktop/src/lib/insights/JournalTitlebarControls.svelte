<script lang="ts">
  // Journal destination chrome (page 08): the date capsule lives in the title
  // bar, not the content — the same glass capsule idiom the Timeline uses. The
  // layout mounts this on /journal; the viewed day is shared with the surface
  // through the `journalDate` store (journal-date.svelte.ts).
  import JournalDateStepper from "$lib/insights/JournalDateStepper.svelte";
</script>

<JournalDateStepper />
