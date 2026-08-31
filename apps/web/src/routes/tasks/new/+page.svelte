<script lang="ts">
  import { goto } from "$app/navigation";
  import type { TaskSummary } from "$lib/contracts";
  import { api, ApiClientError } from "$lib/api";
  import Icon from "$lib/components/Icon.svelte";

  let title = $state("");
  let goal = $state("");
  let error = $state("");
  let submitting = $state(false);

  async function createTask(event: SubmitEvent) {
    event.preventDefault();
    error = "";
    submitting = true;
    try {
      const task = await api<TaskSummary>("/api/tasks", {
        method: "POST",
        body: JSON.stringify({ title, goal })
      });
      await goto(`/runs/${task.latestRunId}`);
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not create task";
    } finally {
      submitting = false;
    }
  }

  const charWarning = $derived(goal.length > 0 && goal.length < 12);
</script>

<svelte:head>
  <title>New task · SoloOps</title>
</svelte:head>

<div class="ops-grid-bg p-6">
  <div class="flex items-center gap-2 text-[11px] text-slate-500">
    <a href="/tasks" class="transition-colors hover:text-slate-300">Tasks</a>
    <Icon name="chevron-right" size={10} />
    <span class="text-slate-300">New task</span>
  </div>

  <div class="mt-3 flex items-end justify-between gap-4">
    <div>
      <h1 class="text-2xl font-semibold tracking-tight text-slate-100">Create a task</h1>
      <p class="mt-1.5 text-[13px] leading-relaxed text-slate-400">
        Start a durable Agent run with approval-gated tools, verification, and an auditable report.
      </p>
    </div>
    <div class="hidden shrink-0 items-center gap-2 md:flex">
      <span
        class="rounded-md border border-edge bg-ink-900/70 px-2.5 py-1 font-mono text-[11px] text-slate-500"
      >
        lease: owner-local
      </span>
      <span
        class="rounded-md border border-edge bg-ink-900/70 px-2.5 py-1 font-mono text-[11px] text-slate-500"
      >
        mode: agent
      </span>
    </div>
  </div>

  <div class="mt-6 grid items-start gap-5 lg:grid-cols-[minmax(0,1fr)_280px]">
    <form class="space-y-5" onsubmit={createTask}>
      <label class="block">
        <span class="mb-1.5 block text-[13px] font-medium text-slate-300">Title</span>
        <input
          class="h-10 w-full rounded-lg border border-edge bg-ink-900/80 px-3 text-[13px] text-slate-100 placeholder:text-slate-600 outline-none transition-colors focus:border-mint-400/50"
          bind:value={title}
          maxlength="160"
          placeholder="e.g. Redeploy the compose stack"
          required
        />
      </label>

      <label class="block">
        <span class="mb-1.5 block text-[13px] font-medium text-slate-300">Goal</span>
        <textarea
          class="min-h-52 w-full resize-y rounded-lg border border-edge bg-ink-900/80 px-3 py-2.5 text-[13px] leading-relaxed text-slate-100 placeholder:text-slate-600 outline-none transition-colors focus:border-mint-400/50"
          bind:value={goal}
          maxlength="20000"
          placeholder="Describe the outcome you want, including any constraints…"
          required></textarea>
        <span class="mt-1.5 block text-right font-mono text-[11px] tabular-nums text-slate-600"
          >{goal.length}/20000</span
        >
      </label>

      {#if charWarning}
        <p class="text-[12px] text-amber-300/90">
          A short goal may be underspecified — include verification criteria for best results.
        </p>
      {/if}
      {#if error}<p class="text-[13px] text-red-300">{error}</p>{/if}

      <button
        class="flex items-center gap-1.5 rounded-md bg-mint-400 px-4 py-2 text-[13px] font-semibold text-ink-950 transition-all hover:bg-mint-500 active:scale-[0.98] disabled:opacity-50"
        disabled={submitting}
      >
        {#if submitting}
          <span
            class="size-3.5 animate-spin rounded-full border-2 border-ink-950/20 border-t-ink-950"
          ></span>
          Creating…
        {:else}
          <Icon name="play" size={13} />
          Create task
        {/if}
      </button>
    </form>

    <aside class="space-y-4">
      <div class="panel p-4">
        <p class="text-[11px] font-semibold uppercase tracking-[0.12em] text-slate-500">
          What happens next
        </p>
        <ol class="mt-3 space-y-3">
          {#each [{ icon: "spark", text: "Agent plans the work and updates the plan panel." }, { icon: "terminal", text: "Tools run approval-gated, one at a time." }, { icon: "file", text: "Evidence and a final report are produced." }] as step (step.text)}
            <li class="flex gap-2.5">
              <span
                class="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md border border-white/10 bg-white/[0.03] text-slate-400"
              >
                <Icon name={step.icon as "spark" | "terminal" | "file"} size={12} />
              </span>
              <span class="text-[12px] leading-relaxed text-slate-400">{step.text}</span>
            </li>
          {/each}
        </ol>
      </div>
      <div class="panel p-4">
        <p
          class="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.12em] text-slate-500"
        >
          <Icon name="shield" size={12} />
          Safety
        </p>
        <p class="mt-2 text-[12px] leading-relaxed text-slate-400">
          Every workspace write, process and network action passes policy. Nothing runs with root.
        </p>
      </div>
      <div class="panel p-4">
        <p class="text-[11px] font-semibold uppercase tracking-[0.12em] text-slate-500">
          Default budget
        </p>
        <div class="command-strip mt-3">
          <div class="px-3 py-2">
            <p class="text-[9.5px] font-medium uppercase tracking-[0.12em] text-slate-600">Turns</p>
            <p class="mt-0.5 font-mono text-[12.5px] text-slate-300">8</p>
          </div>
          <div class="px-3 py-2">
            <p class="text-[9.5px] font-medium uppercase tracking-[0.12em] text-slate-600">Tools</p>
            <p class="mt-0.5 font-mono text-[12.5px] text-slate-300">6</p>
          </div>
          <div class="px-3 py-2">
            <p class="text-[9.5px] font-medium uppercase tracking-[0.12em] text-slate-600">
              Duration
            </p>
            <p class="mt-0.5 font-mono text-[12.5px] text-slate-300">60s</p>
          </div>
        </div>
        <p class="mt-2.5 text-[11px] leading-relaxed text-slate-500">
          Budgets are enforced server-side on every run.
        </p>
      </div>
    </aside>
  </div>
</div>
