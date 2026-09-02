<script lang="ts">
  import type { EvidenceSummary } from "$lib/contracts";
  import Icon from "./Icon.svelte";

  let { items }: { items: EvidenceSummary[] } = $props();
</script>

<section class="panel p-4">
  <div class="flex items-center gap-2">
    <span class="text-muted-foreground/65"><Icon name="file" size={14} /></span>
    <h2 class="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
      Evidence
    </h2>
    <span class="ml-auto font-mono text-2xs text-muted-foreground/65">{items.length}</span>
  </div>

  <ul class="mt-3 space-y-2">
    {#each items as item, index (item.id)}
      <li
        class="animate-fade-up rounded-lg border border-edge bg-ink-800/50 p-3 transition-colors duration-200 hover:border-edge"
        style={`animation-delay:${Math.min(index * 60, 300)}ms`}
      >
        <div class="flex items-center gap-2">
          <span
            class="rounded-full border border-sky-400/25 bg-sky-400/10 px-2 py-0.5 text-2xs font-medium leading-4 text-sky-300"
          >
            {item.kind}
          </span>
          <span class="ml-auto font-mono text-2xs text-muted-foreground/65"
            >rev #{item.workspaceRevision}</span
          >
        </div>
        <p class="mt-2 text-sm leading-relaxed text-muted-foreground">{item.summary}</p>
        <p class="mt-1.5 break-all font-mono text-2xs text-muted-foreground/65" title={item.id}>
          {item.id}
        </p>
      </li>
    {/each}
  </ul>
</section>
