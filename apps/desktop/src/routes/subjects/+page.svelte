<script lang="ts">
  // Subjects — a destination opened from Overview's Subjects tile (⌃J).
  //
  // The titlebar grows a breadcrumb chip and `esc` returns: from the detail to
  // the index, from the index to Overview. Detail is a drill-in over local
  // state on this same route — this app is a static-adapter SPA, so a dynamic
  // route segment would buy nothing.
  //
  // This shell is thin on purpose: the index and the detail each own their own
  // loads, keyboard and deck publication.
  import { goto } from "$app/navigation";
  import { resetCrumbs, setCrumbs } from "$lib/crumb.svelte";
  import SubjectsIndex from "$lib/subjects/SubjectsIndex.svelte";
  import SubjectDetail from "$lib/subjects/SubjectDetail.svelte";

  let openSubject = $state<string | null>(null);

  $effect(() => {
    setCrumbs(
      openSubject
        ? [{ label: "Subjects", href: "/subjects" }, { label: openSubject }]
        : [{ label: "Subjects" }],
    );
    return resetCrumbs;
  });
</script>

<div class="subjects-route">
  {#if openSubject}
    <SubjectDetail subject={openSubject} onBack={() => (openSubject = null)} />
  {:else}
    <SubjectsIndex
      onOpen={(subject) => (openSubject = subject)}
      onExit={() => void goto("/overview")}
    />
  {/if}
</div>

<style>
  /* WKWebView: fill with flex, never height:100% (the phase-1 trap). */
  .subjects-route {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
