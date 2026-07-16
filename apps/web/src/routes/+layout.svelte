<script lang="ts">
  import "../app.css";
  let { children } = $props();
</script>

<svelte:head>
  <title>SoloOps</title>
  <meta name="description" content="Private, controlled agent operations" />
</svelte:head>

<div class="mx-auto min-h-screen max-w-6xl px-5 py-6 sm:px-8">
  <header class="mb-10 flex items-center justify-between border-b border-white/10 pb-5">
    <a class="flex items-center gap-3 text-lg font-semibold tracking-tight" href="/tasks">
      <span class="grid size-8 place-items-center rounded-lg bg-mint-400 font-black text-ink-950">S</span>
      SoloOps
    </a>
    <span class="rounded-full border border-mint-400/30 bg-mint-400/10 px-3 py-1 text-xs font-medium text-mint-400">
      Phase 0
    </span>
  </header>
  {@render children()}
</div>

