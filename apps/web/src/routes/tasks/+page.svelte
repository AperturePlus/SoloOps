<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import type { TaskSummary } from "$lib/contracts";
  import { api, ApiClientError } from "$lib/api";
  import { relativeTime } from "$lib/time";
  import Icon from "$lib/components/Icon.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";

  let tasks = $state<TaskSummary[]>([]);
  let loading = $state(true);
  let error = $state("");
  let selectedId = $state<string | null>(null);
  let query = $state("");
  let listOpen = $state(true);

  onMount(async () => {
    try {
      const response = await api<{ items: TaskSummary[] }>("/api/tasks");
      tasks = response.items;
      selectedId = tasks[0]?.latestRunId ?? null;
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not load tasks";
    } finally {
      loading = false;
    }
  });

  const filtered = $derived(
    query.trim()
      ? tasks.filter(
          (task) =>
            task.title.toLowerCase().includes(query.trim().toLowerCase()) ||
            task.goal.toLowerCase().includes(query.trim().toLowerCase())
        )
      : tasks
  );

  const selected = $derived(tasks.find((task) => task.latestRunId === selectedId) ?? null);

  const activeCount = $derived(
    tasks.filter((task) => !["succeeded", "failed", "cancelled"].includes(task.status)).length
  );

  const succeededCount = $derived(tasks.filter((task) => task.status === "succeeded").length);
  const failedCount = $derived(tasks.filter((task) => task.status === "failed").length);
  const runningCount = $derived(tasks.filter((task) => task.status === "running").length);

  async function logout() {
    await api("/api/auth/logout", { method: "POST" });
    await goto("/login");
  }

  function statusDotClass(status: string): string {
    if (status === "blocked" || status === "failed" || status === "needs_recovery")
      return "bg-amber-400";
    if (status === "succeeded") return "bg-mint-400";
    if (status === "cancelled") return "bg-slate-600";
    if (status === "waiting_for_approval") return "bg-amber-300";
    return "bg-sky-400";
  }

  function relative(ts: number): string {
    return relativeTime(ts);
  }
</script>

<svelte:head>
  <title>Tasks · SoloOps</title>
</svelte:head>

<div class="flex h-screen overflow-hidden">
  <!-- ============ Left: task list rail ============ -->
  <aside
    class={`relative flex shrink-0 flex-col border-r border-edge bg-ink-950/50 transition-all duration-300 ${
      listOpen ? "w-80" : "w-14"
    }`}
  >
    <div class="hairline flex h-12 shrink-0 items-center gap-2 px-3">
      {#if listOpen}
        <span class="min-w-0 flex-1 truncate text-sm font-semibold tracking-tight text-slate-100">
          Tasks
        </span>
        <span class="font-mono text-[11px] text-slate-500">{tasks.length}</span>
      {/if}
      <button
        class="grid size-7 shrink-0 place-items-center rounded-md text-slate-500 transition-colors hover:bg-white/5 hover:text-slate-200"
        aria-label={listOpen ? "Collapse task list" : "Expand task list"}
        onclick={() => (listOpen = !listOpen)}
      >
        <Icon name={listOpen ? "chevron-down" : "chevron-right"} size={14} />
      </button>
    </div>

    {#if listOpen}
      <div class="px-2 pt-1">
        <div class="relative">
          <span class="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-slate-600">
            <Icon name="search" size={13} />
          </span>
          <input
            class="h-8 w-full rounded-md border border-edge bg-ink-900/70 pl-7 pr-2 text-xs text-slate-200 placeholder:text-slate-600 outline-none focus:border-mint-400/40"
            placeholder="Filter tasks…"
            bind:value={query}
          />
        </div>
      </div>
    {/if}

    <div class="agent-scroll min-h-0 flex-1 overflow-y-auto p-2">
      {#if loading}
        <div class="space-y-2 p-1">
          <div class="h-16 animate-pulse rounded-lg bg-white/[0.04]"></div>
          <div class="h-16 animate-pulse rounded-lg bg-white/[0.04]"></div>
          <div class="h-16 animate-pulse rounded-lg bg-white/[0.04]"></div>
        </div>
      {:else if filtered.length === 0}
        {#if listOpen}
          <div class="mt-8 px-4 text-center">
            <p class="text-sm text-slate-500">No tasks</p>
            <p class="mt-1 text-xs text-slate-600">
              {query ? "No match for the filter." : "Create your first goal to start."}
            </p>
          </div>
        {/if}
      {:else}
        <ul class="space-y-1">
          {#each filtered as task (task.id)}
            {@const activeSelection = task.latestRunId === selectedId}
            <li>
              <button
                type="button"
                class={`group flex w-full items-start gap-2.5 rounded-lg border px-2.5 py-2 text-left transition-colors duration-150 ${
                  activeSelection
                    ? "border-edge-strong bg-ink-800/70"
                    : "border-transparent hover:bg-white/[0.03]"
                }`}
                onclick={() => {
                  selectedId = task.latestRunId;
                  if (!listOpen) listOpen = true;
                }}
              >
                <span
                  class={`mt-1.5 size-1.5 shrink-0 rounded-full ${statusDotClass(task.status)}`}
                  aria-hidden="true"
                ></span>
                <span class="min-w-0 flex-1">
                  <span
                    class={`block truncate text-[13px] leading-tight ${
                      activeSelection ? "text-slate-100" : "text-slate-300"
                    }`}
                  >
                    {task.title}
                  </span>
                  <span class="mt-0.5 block truncate text-[11px] text-slate-500">
                    {relative(task.createdAt)}
                  </span>
                </span>
                {#if listOpen}
                  <span
                    class={`mt-1 shrink-0 rounded px-1 py-px font-mono text-[9px] uppercase tracking-wide ${
                      task.status === "succeeded"
                        ? "bg-mint-400/10 text-mint-400"
                        : task.status === "failed"
                          ? "bg-red-400/10 text-red-300"
                          : "bg-sky-400/10 text-sky-300"
                    }`}
                  >
                    {task.status === "succeeded" || task.status === "failed"
                      ? task.status.slice(0, 3)
                      : "live"}
                  </span>
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="hairline shrink-0 border-t px-2 py-2">
      {#if listOpen}
        <div class="mb-2 flex items-center gap-1 text-[11px] text-slate-600">
          <span class="size-1.5 rounded-full bg-mint-400/70"></span>
          <span>{activeCount} active</span>
        </div>
        <div class="flex gap-1">
          <a
            href="/tasks/new"
            class="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-edge bg-ink-900/70 px-2 py-1.5 text-xs text-slate-300 transition-colors hover:border-edge-strong hover:text-slate-100"
          >
            <Icon name="plus" size={12} />
            New task
          </a>
        </div>
      {/if}
      <button
        class="mt-1 grid w-full place-items-center rounded-md p-1.5 text-slate-500 transition-colors hover:bg-white/5 hover:text-red-200"
        aria-label="Sign out"
        title="Sign out"
        onclick={logout}
      >
        <Icon name="logout" size={14} />
      </button>
    </div>
  </aside>

  <!-- ============ Right: detail zone ============ -->
  <div class="ops-grid-bg agent-scroll min-w-0 flex-1 overflow-y-auto">
    {#if loading}
      <div class="mx-auto max-w-3xl space-y-4 p-8">
        <div class="h-8 w-48 animate-pulse rounded-lg bg-white/[0.04]"></div>
        <div class="h-40 animate-pulse rounded-xl bg-white/[0.04]"></div>
        <div class="h-24 animate-pulse rounded-xl bg-white/[0.04]"></div>
      </div>
    {:else if error}
      <div class="p-8 text-sm text-red-300">{error}</div>
    {:else if !selected}
      <div class="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
        <span
          class="grid size-12 place-items-center rounded-xl border border-edge bg-ink-900/60 text-slate-600"
        >
          <Icon name="box" size={22} />
        </span>
        <p class="text-sm text-slate-400">No task selected</p>
        <a
          href="/tasks/new"
          class="mt-1 flex items-center gap-1.5 rounded-md bg-mint-400 px-3 py-1.5 text-xs font-semibold text-ink-950 transition-opacity hover:opacity-90"
        >
          <Icon name="plus" size={12} />
          Create a task
        </a>
      </div>
    {:else}
      <div class="min-h-0 p-5">
        <!-- Breadcrumb-ish heading -->
        <div class="flex items-center gap-2 text-[11px] text-slate-500">
          <span>Execution queue</span>
          <Icon name="chevron-right" size={10} />
          <span class="text-slate-300">{selected.title}</span>
        </div>

        <!-- Title row with inline actions (no max-w shrink) -->
        <div class="mt-2 flex items-start justify-between gap-4">
          <div class="min-w-0">
            <h1 class="truncate text-lg font-semibold tracking-tight text-slate-100">
              {selected.title}
            </h1>
            <p class="mt-1 line-clamp-2 max-w-3xl text-[13px] leading-relaxed text-slate-400">
              {selected.goal}
            </p>
          </div>
          <a
            href={`/runs/${selected.latestRunId}`}
            class="flex shrink-0 items-center gap-1.5 rounded-md bg-mint-400 px-3 py-1.5 text-xs font-semibold text-ink-950 transition-opacity hover:opacity-90"
          >
            Open run
            <Icon name="arrow-up-right" size={12} />
          </a>
        </div>

        <!-- Status + timing strip -->
        <div class="mt-4 flex flex-wrap items-center gap-x-5 gap-y-2">
          <StatusBadge status={selected.status} />
          <span class="flex items-center gap-1.5 font-mono text-[11px] text-slate-500">
            <Icon name="clock" size={12} />
            {new Date(selected.createdAt).toLocaleString()}
          </span>
          <span class="flex items-center gap-1.5 font-mono text-[11px] text-slate-500">
            <Icon name="hash" size={12} />
            {selected.latestRunId.slice(0, 10)}
          </span>
          {#if tasks.length > 1}
            <span class="ml-auto flex items-center gap-1.5 text-[11px] text-slate-500">
              <Icon name="layers" size={12} />
              {tasks.length} tasks in queue
            </span>
          {/if}
        </div>

        <!-- Codex command strip: 4 metrics across the full width -->
        <dl class="mt-4 grid grid-cols-2 gap-2 md:grid-cols-4">
          <div class="metric-chip">
            <dt>Mode</dt>
            <dd>agent</dd>
          </div>
          <div class="metric-chip">
            <dt>Run</dt>
            <dd>{selected.latestRunId.slice(0, 12)}</dd>
          </div>
          <div class="metric-chip">
            <dt>Status</dt>
            <dd class="!text-slate-300 normal-case">{selected.status.replace(/_/g, " ")}</dd>
          </div>
          <div class="metric-chip">
            <dt>Queued</dt>
            <dd class="!text-slate-300 normal-case">{relative(selected.createdAt)}</dd>
          </div>
        </dl>

        <!-- Composite two-zone grid: objective + queue glance -->
        <div class="mt-4 grid gap-4 xl:grid-cols-[minmax(0,1fr)_300px]">
          <!-- Objective (widened) -->
          <section class="panel min-h-0 p-5">
            <div class="flex items-center gap-2">
              <span class="text-slate-500"><Icon name="spark" size={14} /></span>
              <h2 class="text-[13px] font-semibold tracking-tight text-slate-200">Objective</h2>
            </div>
            <p
              class="mt-3 max-h-72 overflow-y-auto whitespace-pre-wrap text-[13px] leading-relaxed text-slate-300 agent-scroll"
            >
              {selected.goal}
            </p>
          </section>

          <!-- Queue glance: right rail with history + status counts (no bottom stack) -->
          <section class="panel min-h-0 p-5">
            <div class="flex items-center justify-between">
              <h2 class="text-[13px] font-semibold tracking-tight text-slate-200">Queue glance</h2>
              <span class="font-mono text-[10px] text-slate-600">{tasks.length} total</span>
            </div>
            <dl class="mt-3 grid grid-cols-3 gap-1.5">
              <div class="rounded-md border border-white/5 bg-white/[0.02] px-2 py-1.5 text-center">
                <dt class="text-[9px] font-medium uppercase tracking-[0.12em] text-slate-600">
                  Running
                </dt>
                <dd class="mt-0.5 font-mono text-sm text-sky-300">{runningCount}</dd>
              </div>
              <div class="rounded-md border border-white/5 bg-white/[0.02] px-2 py-1.5 text-center">
                <dt class="text-[9px] font-medium uppercase tracking-[0.12em] text-slate-600">
                  Done
                </dt>
                <dd class="mt-0.5 font-mono text-sm text-mint-300">{succeededCount}</dd>
              </div>
              <div class="rounded-md border border-white/5 bg-white/[0.02] px-2 py-1.5 text-center">
                <dt class="text-[9px] font-medium uppercase tracking-[0.12em] text-slate-600">
                  Failed
                </dt>
                <dd class="mt-0.5 font-mono text-sm text-red-300">{failedCount}</dd>
              </div>
            </dl>

            <div class="mt-4 flex items-center justify-between">
              <h3 class="text-[11px] font-semibold uppercase tracking-[0.12em] text-slate-500">
                Run history
              </h3>
            </div>
            <ul class="agent-scroll mt-2 max-h-56 space-y-0 overflow-y-auto pr-1">
              {#each tasks.slice(0, 6) as task, index (task.id)}
                <li class="track-line relative pl-4 pb-3 last:pb-0">
                  <span
                    class={`absolute top-1.5 left-0 size-2 -translate-x-1/2 rounded-full border border-ink-950 ${
                      task.latestRunId === selected.latestRunId
                        ? "bg-mint-400"
                        : index % 2 === 0
                          ? "bg-slate-500"
                          : "bg-slate-700"
                    }`}
                    aria-hidden="true"
                  ></span>
                  <a
                    href={`/runs/${task.latestRunId}`}
                    class="block truncate text-xs font-medium text-slate-300 transition-colors hover:text-mint-400"
                  >
                    {task.title}
                  </a>
                  <p class="mt-0.5 flex items-center gap-1.5 text-[11px] text-slate-600">
                    <span class={`size-1 rounded-full ${statusDotClass(task.status)}`}></span>
                    {task.status.replace(/_/g, " ")}
                    <span class="ml-auto font-mono tabular-nums">{relative(task.createdAt)}</span>
                  </p>
                </li>
              {/each}
            </ul>
          </section>
        </div>
      </div>
    {/if}
  </div>
</div>
