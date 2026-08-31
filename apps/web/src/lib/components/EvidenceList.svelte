<script lang="ts">
  import type { EvidenceSummary } from "$lib/contracts";
  import Icon from "./Icon.svelte";

  let { items }: { items: EvidenceSummary[] } = $props();
</script>

<section class="panel p-4">
  <div class="flex items-center gap-2">
    <span class="text-slate-500"><Icon name="file" size={13} /></span>
    <h2 class="text-xs font-semibold uppercase tracking-[0.12em] text-slate-400">Evidence</h2>
    <span class="ml-auto font-mono text-[11px] text-slate-500">{items.length}</span>
  </div>

  <ul class="mt-3 space-y-2">
    {#each items as item, index (item.id)}
      <li
        class="animate-fade-up rounded-lg border border-white/5 bg-white/[0.02] p-3 transition-colors duration-200 hover:border-white/15"
        style={`animation-delay:${Math.min(index * 60, 300)}ms`}
      >
        <div class="flex items-center gap-2">
          <span
            class="rounded-full border border-sky-400/25 bg-sky-400/10 px-2 py-0.5 text-[10px] font-medium leading-4 text-sky-300"
          >
            {item.kind}
          </span>
          <span class="ml-auto font-mono text-[10px] text-slate-600"
            >rev #{item.workspaceRevision}</span
          >
        </div>
        <p class="mt-2 text-[13px] leading-relaxed text-slate-300">{item.summary}</p>
        <p class="mt-1.5 break-all font-mono text-[10px] text-slate-600" title={item.id}>
          {item.id}
        </p>
      </li>
    {/each}
  </ul>
</section>