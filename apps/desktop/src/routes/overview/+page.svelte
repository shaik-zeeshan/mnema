<script lang="ts">
  // Overview — Timeline's peer surface (⌘2). This is the scaffold only: the
  // three sections below are the shape G11 settled (This Week bars + Ask
  // history in, Open Threads as digest prose), and their content arrives with
  // the Overview data endpoints slice. Deliberately no tiles and no placeholder
  // numbers — a fake bar chart would be worse than an empty heading.
  const sections = [
    {
      title: "This Week",
      note: "Seven days of capture, one bar per day.",
    },
    {
      title: "Recent asks",
      note: "The questions you asked Mnema, newest first.",
    },
    {
      title: "Open threads",
      note: "The one thread the daily digest says is still open.",
    },
  ];
</script>

<div class="overview">
  <header class="overview__head">
    <h1 class="overview__title">Overview</h1>
    <p class="overview__lede">
      The read-at-a-glance surface. Not wired up yet — Timeline still has everything.
    </p>
  </header>

  <div class="overview__sections">
    {#each sections as section (section.title)}
      <section class="overview__section">
        <h2 class="overview__section-title">{section.title}</h2>
        <p class="overview__section-note">{section.note}</p>
        <p class="overview__empty">Nothing here yet.</p>
      </section>
    {/each}
  </div>
</div>

<style>
  .overview {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 28px 32px;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }

  .overview__head {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .overview__title {
    margin: 0;
    font-size: var(--t-title);
    font-weight: 590;
    color: var(--app-text-strong);
  }

  .overview__lede {
    margin: 0;
    font-size: var(--t-ui);
    color: var(--app-text-muted);
  }

  .overview__sections {
    display: flex;
    flex-direction: column;
    gap: 20px;
    max-width: 720px;
  }

  .overview__section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .overview__section-title {
    margin: 0;
    font-size: var(--t-read);
    font-weight: 590;
    color: var(--app-text-strong);
  }

  .overview__section-note {
    margin: 0;
    font-size: var(--t-meta);
    color: var(--app-text-muted);
  }

  .overview__empty {
    margin: 8px 0 0;
    padding: 20px;
    border: 1px dashed var(--app-border);
    border-radius: 8px;
    font-size: var(--t-ui);
    color: var(--app-text-faint);
    text-align: center;
  }
</style>
