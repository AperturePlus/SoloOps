<script lang="ts">
  import type { BudgetSnapshot, UsageSnapshot } from "$lib/contracts";

  let {
    budget,
    usage,
    workspaceRevision = 0
  }: { budget: BudgetSnapshot; usage: UsageSnapshot; workspaceRevision?: number } = $props();

  type Meter = { label: string; used: number; max: number };

  const meters = $derived.by(() => {
    const base: Meter[] = [
      { label: "Turns", used: usage.modelTurns, max: budget.maxModelTurns },
      { label: "Tools", used: usage.toolCalls, max: budget.maxToolCalls },
      { label: "In tok", used: usage.inputTokens, max: budget.maxInputTokens },
      { label: "Out tok", used: usage.outputTokens, max: budget.maxOutputTokens }
    ];
    return base.map((meter) => {
      const ratio = meter.max > 0 ? meter.used / meter.max : 0;
      const tone = ratio >= 0.95 ? "bg-red-400" : ratio >= 0.8 ? "bg-amber-400" : "bg-mint-400";
      return { ...meter, percent: Math.min(100, Math.round(ratio * 100)), tone };
    });
  });

  function formatTokens(value: number): string {
    if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
    if (value >= 10_000) return `${Math.round(value / 1000)}k`;
    return String(value);
  }
</script>

<section class="panel p-4">
  <div class="flex items-center justify-between gap-3">
    <h2 class="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">Budget</h2>
    {#if usage.elapsedMs > 0}
      <span class="font-mono text-2xs text-muted-foreground/65"
        >elapsed {Math.round(usage.elapsedMs / 1000)}s</span
      >
    {/if}
  </div>

  <dl class="mt-3 space-y-2.5">
    {#each meters as meter (meter.label)}
      <div class="flex items-center gap-3">
        <dt class="w-14 shrink-0 text-2xs text-muted-foreground/65">{meter.label}</dt>
        <div
          class="h-1 min-w-0 flex-1 overflow-hidden rounded-full bg-ink-800/80"
          aria-hidden="true"
        >
          <div
            class={`h-full rounded-full transition-all duration-700 ease-out ${meter.tone}`}
            style={`width:${meter.percent}%`}
          ></div>
        </div>
        <dd class="w-16 shrink-0 text-right font-mono text-2xs tabular-nums text-muted-foreground">
          {meter.max > 0 ? formatTokens(meter.used) : meter.used}
          {#if meter.max > 0}
            <span class="text-muted-foreground/65">/{formatTokens(meter.max)}</span>
          {/if}
        </dd>
      </div>
    {/each}
  </dl>

  <p
    class="mt-3 flex flex-wrap gap-x-4 gap-y-1 border-t border-edge pt-2.5 text-2xs text-muted-foreground/65"
  >
    <span>cached {formatTokens(usage.cachedInputTokens)}</span>
    <span>writes {formatTokens(usage.cacheWriteInputTokens)}</span>
    <span>workspace rev #{workspaceRevision}</span>
  </p>
</section>
