<script lang="ts">
  import type { AgentPlan, PlanStepStatus } from "$lib/contracts";

  let { plan }: { plan: AgentPlan } = $props();

  const completed = $derived(plan.steps.filter((step) => step.status === "completed").length);
  const total = $derived(plan.steps.length);
  const percent = $derived(total > 0 ? Math.round((completed / total) * 100) : 0);

  function stepTone(status: PlanStepStatus): string {
    switch (status) {
      case "completed":
        return "border-mint-400/40 bg-mint-400/10 text-mint-400";
      case "in_progress":
        return "border-sky-400/40 bg-sky-400/10 text-sky-300";
      case "blocked":
        return "border-amber-400/40 bg-amber-400/10 text-amber-300";
      default:
        return "border-edge bg-ink-800/60 text-muted-foreground/65";
    }
  }

  const STATUS_LABELS: Record<PlanStepStatus, string> = {
    pending: "pending",
    in_progress: "in progress",
    completed: "done",
    blocked: "blocked"
  };
</script>

<section class="panel p-4">
  <div class="flex items-center justify-between gap-3">
    <h2 class="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">Plan</h2>
    {#if total > 0}
      <span class="font-mono text-2xs text-muted-foreground/65">{completed}/{total} steps</span>
    {/if}
  </div>

  {#if plan.summary}
    <p class="mt-2 text-sm leading-relaxed text-muted-foreground">{plan.summary}</p>
  {/if}

  {#if total > 0}
    <div class="mt-3 h-1 overflow-hidden rounded-full bg-ink-800/80" aria-hidden="true">
      <div
        class="h-full rounded-full bg-mint-400 transition-all duration-700 ease-out"
        style={`width:${percent}%`}
      ></div>
    </div>

    <ol class="mt-3 space-y-1">
      {#each plan.steps as step, index (step.id)}
        <li
          class="animate-fade-up flex items-center gap-2.5 rounded-md px-1.5 py-1 transition-colors duration-200 hover:bg-ink-800/60"
          style={`animation-delay:${Math.min(index * 60, 300)}ms`}
        >
          <span
            class={`grid size-4.5 shrink-0 place-items-center rounded-full border text-2xs font-semibold transition-colors duration-300 ${stepTone(
              step.status
            )}`}
          >
            {#if step.status === "completed"}
              <svg
                viewBox="0 0 12 12"
                class="size-2.5"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M2 6.5 4.5 9 10 3" />
              </svg>
            {:else if step.status === "in_progress"}
              <span class="size-1.5 animate-pulse rounded-full bg-current"></span>
            {:else}
              {index + 1}
            {/if}
          </span>
          <span
            class={`min-w-0 flex-1 truncate text-sm transition-colors duration-300 ${
              step.status === "completed"
                ? "text-muted-foreground/65 line-through decoration-foreground/20"
                : "text-foreground"
            }`}
          >
            {step.title}
            {#if !step.required}
              <span class="ml-1 text-2xs text-muted-foreground/65">optional</span>
            {/if}
          </span>
          <span class="shrink-0 text-2xs text-muted-foreground/65"
            >{STATUS_LABELS[step.status]}</span
          >
        </li>
      {/each}
    </ol>
  {:else}
    <p class="mt-3 text-sm text-muted-foreground/65">No plan yet.</p>
  {/if}
</section>
