<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    index,
    title,
    subtitle,
    children,
  }: {
    index: string;
    title: string;
    subtitle: string;
    children: Snippet;
  } = $props();
</script>

<section class="card">
  <div class="card__header">
    <div class="card__heading">
      <span class="card__index">{index}</span>
      <div>
        <h2 class="card__title">{title}</h2>
        <p class="card__subtitle">{subtitle}</p>
      </div>
    </div>
  </div>
  {@render children()}
</section>
