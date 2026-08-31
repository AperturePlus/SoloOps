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
        return "border-white/10 bg-white/[0.03] text-slate-500";
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
    <h2 class="text-xs font-semibold uppercase tracking-[0.12em] text-slate-400">Plan</h2>
    {#if total > 0}
      <span class="font-mono text-[11px] text-slate-500">{completed}/{total} steps</span>
    {/if}
  </div>

  {#if plan.summary}
    <p class="mt-2 text-[13px] leading-relaxed text-slate-300">{plan.summary}</p>
  {/if}

  {#if total > 0}
    <div class="mt-3 h-1 overflow-hidden rounded-full bg-white/[0.06]" aria-hidden="true">
      <div
        class="h-full rounded-full bg-mint-400 transition-all duration-700 ease-out"
        style={`width:${percent}%`}
      ></div>
    </div>

    <ol class="mt-3 space-y-1">
      {#each plan.steps as step, index (step.id)}
        <li
          class="animate-fade-up flex items-center gap-2.5 rounded-md px-1.5 py-1 transition-colors duration-200 hover:bg-white/[0.03]"
          style={`animation-delay:${Math.min(index * 60, 300)}ms`}
        >
          <span
            class={`grid size-4.5 shrink-0 place-items-center rounded-full border text-[9px] font-semibold transition-colors duration-300 ${stepTone(
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
            class={`min-w-0 flex-1 truncate text-[13px] transition-colors duration-300 ${
              step.status === "completed" ? "text-slate-500 line-through decoration-slate-700" : "text-slate-200"
            }`}
          >
            {step.title}
            {#if !step.required}
              <span class="ml-1 text-[11px] text-slate-600">optional</span>
            {/if}
          </span>
          <span class="shrink-0 text-[11px] text-slate-600">{STATUS_LABELS[step.status]}</span>
        </li>
      {/each}
    </ol>
  {:else}
    <p class="mt-3 text-[13px] text-slate-500">No plan yet.</p>
  {/if}
</section>