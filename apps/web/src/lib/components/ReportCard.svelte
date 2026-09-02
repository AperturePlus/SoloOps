<script lang="ts">
  import type { FinalReport } from "$lib/contracts";
  import { formatDuration } from "$lib/time";
  import Icon from "./Icon.svelte";

  let { report }: { report: FinalReport } = $props();

  const OUTCOME_TONES: Record<string, string> = {
    succeeded: "border-mint-400/30 bg-mint-400/10 text-mint-400",
    failed: "border-red-400/30 bg-red-400/10 text-red-300",
    partial: "border-amber-400/30 bg-amber-400/10 text-amber-300"
  };

  const outcomeTone = $derived(OUTCOME_TONES[report.outcome] ?? OUTCOME_TONES.partial);
</script>

<section class="panel border-mint-400/20 bg-mint-400/[0.03] p-4">
  <div class="flex flex-wrap items-center gap-3">
    <span class="text-mint-400"><Icon name="check" size={14} /></span>
    <h2 class="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
      Final report
    </h2>
    <span class={`rounded-full border px-2 py-0.5 text-2xs font-medium capitalize ${outcomeTone}`}>
      {report.outcome}
    </span>
    {#if report.usage?.elapsedMs}
      <span class="ml-auto font-mono text-2xs tabular-nums text-muted-foreground/65">
        {formatDuration(report.usage.elapsedMs)} · {report.usage.modelTurns} turns ·
        {report.usage.toolCalls} tools
      </span>
    {/if}
  </div>

  {#if report.summary}
    <p class="mt-3 text-sm leading-relaxed text-foreground">{report.summary}</p>
  {/if}

  {#if report.completed.length || report.incomplete.length || report.risks.length}
    <div class="mt-4 grid gap-4 sm:grid-cols-3">
      {#if report.completed.length}
        <div>
          <p class="flex items-center gap-1.5 text-xs font-semibold text-mint-400">
            <Icon name="check" size={12} />
            Completed
          </p>
          <ul class="mt-2 space-y-1 text-xs leading-relaxed text-muted-foreground">
            {#each report.completed as item (item)}<li>{item}</li>{/each}
          </ul>
        </div>
      {/if}
      {#if report.incomplete.length}
        <div>
          <p class="flex items-center gap-1.5 text-xs font-semibold text-amber-300">
            <Icon name="circle" size={12} />
            Incomplete
          </p>
          <ul class="mt-2 space-y-1 text-xs leading-relaxed text-muted-foreground">
            {#each report.incomplete as item (item)}<li>{item}</li>{/each}
          </ul>
        </div>
      {/if}
      {#if report.risks.length}
        <div>
          <p class="flex items-center gap-1.5 text-xs font-semibold text-red-300">
            <Icon name="triangle" size={12} />
            Risks
          </p>
          <ul class="mt-2 space-y-1 text-xs leading-relaxed text-muted-foreground">
            {#each report.risks as item (item)}<li>{item}</li>{/each}
          </ul>
        </div>
      {/if}
    </div>
  {/if}

  {#if report.rollback}
    <p
      class="mt-4 rounded-lg border border-edge bg-black/25 p-3 text-xs leading-relaxed text-muted-foreground"
    >
      <span class="font-semibold text-muted-foreground">Rollback:</span>
      {report.rollback}
    </p>
  {/if}

  {#if report.markdown}
    <details class="group mt-3">
      <summary
        class="flex cursor-pointer items-center gap-1.5 text-xs text-muted-foreground/65 transition-colors duration-200 hover:text-muted-foreground"
      >
        <Icon name="chevron-right" size={12} />
        Full markdown report
      </summary>
      <pre
        class="agent-scroll mt-2 max-h-96 overflow-auto whitespace-pre-wrap rounded-lg border border-edge bg-black/25 p-3 text-xs leading-relaxed text-muted-foreground">{report.markdown}</pre>
    </details>
  {/if}
</section>
