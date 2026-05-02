<script lang="ts">
  import { developerOptions, loadDeveloperOptions } from "$lib/developer-options.svelte";

  const devEnabled = $derived(developerOptions.value);

  $effect(() => {
    loadDeveloperOptions();
  });
</script>

<section class="menu">
  <header class="menu__head">
    <span class="menu__eyebrow">capture · z</span>
    <h1 class="menu__title">Menu</h1>
    <p class="menu__sub">Configure the recorder or jump into developer surfaces.</p>
  </header>

  <nav class="menu__cards" aria-label="Application sections">
    <a class="card card--primary" href="/settings">
      <span class="card__row">
        <span class="card__icon" aria-hidden="true">⊙</span>
        <span class="card__chev" aria-hidden="true">→</span>
      </span>
      <span class="card__title">Settings</span>
      <span class="card__desc">
        Capture sources, resolution, bitrate, microphone behavior, and storage.
      </span>
    </a>

    {#if devEnabled}
      <a class="card" href="/debug">
        <span class="card__row">
          <span class="card__icon card__icon--debug" aria-hidden="true">◉</span>
          <span class="card__chev" aria-hidden="true">→</span>
        </span>
        <span class="card__title">Debug</span>
        <span class="card__desc">
          Inspect the live capture pipeline, frame batches, and inactivity signals.
        </span>
      </a>
    {/if}
  </nav>

  <a class="menu__back" href="/">
    <span aria-hidden="true">▦</span>
    <span>Back to timeline</span>
  </a>
</section>

<style>
  .menu {
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding-top: 16px;
  }

  .menu__head {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .menu__eyebrow {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: #44445a;
  }

  .menu__title {
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: #e2e2e8;
  }

  .menu__sub {
    font-size: 12px;
    color: #6a6a88;
    max-width: 44ch;
  }

  .menu__cards {
    display: grid;
    grid-template-columns: 1fr;
    gap: 10px;
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 16px 18px;
    background: #0e0e16;
    border: 1px solid #1a1a2a;
    border-radius: 8px;
    color: #c0c0d8;
    transition: background 0.15s, border-color 0.15s, transform 0.15s;
  }

  .card:hover {
    background: #13131e;
    border-color: #2a2a3e;
  }

  .card:active {
    transform: translateY(1px);
  }

  .card:focus-visible {
    outline: none;
    border-color: #3dffa0;
    box-shadow: 0 0 0 2px rgba(61, 255, 160, 0.18);
  }

  .card--primary {
    background: linear-gradient(180deg, #11111c 0%, #0e0e16 100%);
    border-color: #1e1e2e;
  }

  .card__row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: #44445a;
    font-size: 12px;
  }

  .card__icon {
    color: #3dffa0;
    font-size: 11px;
    letter-spacing: 0.1em;
  }

  .card__icon--debug {
    color: #ff8855;
  }

  .card__chev {
    font-size: 13px;
    color: #44445a;
    transition: transform 0.15s, color 0.15s;
  }

  .card:hover .card__chev {
    color: #c0c0d8;
    transform: translateX(2px);
  }

  .card__title {
    font-size: 14px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: #e2e2e8;
  }

  .card__desc {
    font-size: 11.5px;
    line-height: 1.55;
    color: #6a6a88;
  }

  .menu__back {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    align-self: flex-start;
    padding: 6px 10px;
    margin-top: 4px;
    border-radius: 4px;
    color: #44445a;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    border: 1px solid transparent;
    transition: color 0.12s, background 0.12s, border-color 0.12s;
  }

  .menu__back:hover {
    color: #c0c0d8;
    background: #13131e;
    border-color: #1e1e2e;
  }

  .menu__back:focus-visible {
    outline: none;
    color: #c0c0d8;
    border-color: #3dffa0;
    box-shadow: 0 0 0 2px rgba(61, 255, 160, 0.18);
  }
</style>
