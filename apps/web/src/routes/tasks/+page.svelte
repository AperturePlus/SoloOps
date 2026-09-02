<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import type { RuntimeSnapshot, TaskSummary } from "$lib/contracts";
  import { api, ApiClientError } from "$lib/api";
  import { formatDateTime, relativeTime } from "$lib/time";
  import Icon from "$lib/components/Icon.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import PlanPanel from "$lib/components/PlanPanel.svelte";
  import BudgetPanel from "$lib/components/BudgetPanel.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { Button } from "$lib/components/ui/button";

  let tasks = $state<TaskSummary[]>([]);
  let runtime = $state<RuntimeSnapshot | null>(null);
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
      await loadRuntime(selectedId);
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not load tasks";
    } finally {
      loading = false;
    }
  });

  async function loadRuntime(runId: string | null) {
    runtime = null;
    if (!runId) return;
    try {
      runtime = await api<RuntimeSnapshot>(`/api/runs/${encodeURIComponent(runId)}/runtime`);
    } catch {
      runtime = null;
    }
  }

  function select(runId: string) {
    if (runId === selectedId) return;
    selectedId = runId;
    if (!listOpen) listOpen = true;
    void loadRuntime(runId);
  }

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
    if (status === "cancelled") return "bg-muted-foreground/65";
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
        <span class="min-w-0 flex-1 truncate text-sm font-semibold tracking-tight text-foreground">
          Tasks
        </span>
        <span class="font-mono text-2xs text-muted-foreground/65">{tasks.length}</span>
      {/if}
      <button
        class="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground/65 transition-colors hover:bg-ink-800/70 hover:text-foreground"
        aria-label={listOpen ? "Collapse task list" : "Expand task list"}
        onclick={() => (listOpen = !listOpen)}
      >
        <Icon name={listOpen ? "chevron-down" : "chevron-right"} size={14} />
      </button>
    </div>

    {#if listOpen}
      <div class="px-2 pt-1">
        <div class="relative">
          <span
            class="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted-foreground/65"
          >
            <Icon name="search" size={14} />
          </span>
          <input
            class="h-8 w-full rounded-md border border-edge bg-ink-900/70 pl-7 pr-2 text-xs text-foreground placeholder:text-muted-foreground/65 outline-none focus:border-mint-400/40"
            placeholder="Filter tasks…"
            bind:value={query}
          />
        </div>
      </div>
    {/if}

    <div class="agent-scroll min-h-0 flex-1 overflow-y-auto p-2">
      {#if loading}
        <div class="space-y-2 p-1">
          <div class="h-16 animate-pulse rounded-lg bg-ink-800/70"></div>
          <div class="h-16 animate-pulse rounded-lg bg-ink-800/70"></div>
          <div class="h-16 animate-pulse rounded-lg bg-ink-800/70"></div>
        </div>
      {:else if filtered.length === 0}
        {#if listOpen}
          <div class="mt-8 px-4 text-center">
            <p class="text-sm text-muted-foreground/65">No tasks</p>
            <p class="mt-1 text-xs text-muted-foreground/65">
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
                    : "border-transparent hover:bg-ink-800/60"
                }`}
                onclick={() => select(task.latestRunId)}
              >
                <span
                  class={`mt-1.5 size-1.5 shrink-0 rounded-full ${statusDotClass(task.status)}`}
                  aria-hidden="true"
                ></span>
                <span class="min-w-0 flex-1">
                  <span
                    class={`block truncate text-sm leading-tight ${
                      activeSelection ? "text-foreground" : "text-muted-foreground"
                    }`}
                  >
                    {task.title}
                  </span>
                  <span class="mt-0.5 block truncate text-2xs text-muted-foreground/65">
                    {relative(task.createdAt)}
                  </span>
                </span>
                {#if listOpen}
                  <StatusBadge status={task.status} compact />
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="hairline shrink-0 border-t px-2 py-2">
      {#if listOpen}
        <div class="mb-2 flex items-center gap-1 text-2xs text-muted-foreground/65">
          <span class="size-1.5 rounded-full bg-mint-400/70"></span>
          <span>{activeCount} active</span>
        </div>
        <div class="flex gap-1">
          <a
            href="/tasks/new"
            class="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-edge bg-ink-900/70 px-2 py-1.5 text-xs text-muted-foreground transition-colors hover:border-edge-strong hover:text-foreground"
          >
            <Icon name="plus" size={12} />
            New task
          </a>
        </div>
      {/if}
      <button
        type="button"
        class="mt-1 flex w-full items-center justify-center gap-1.5 rounded-md px-2 py-1.5 text-2xs text-muted-foreground/65 transition-colors hover:text-destructive"
        aria-label="Sign out"
        title="Sign out"
        onclick={logout}
      >
        <Icon name="logout" size={12} />
        Sign out
      </button>
    </div>
  </aside>

  <!-- ============ Right: detail zone ============ -->
  <div class="ops-grid-bg agent-scroll min-w-0 flex-1 overflow-y-auto">
    {#if loading}
      <div class="mx-auto max-w-3xl space-y-4 p-8">
        <div class="h-8 w-48 animate-pulse rounded-lg bg-ink-800/70"></div>
        <div class="h-40 animate-pulse rounded-lg bg-ink-800/70"></div>
        <div class="h-24 animate-pulse rounded-lg bg-ink-800/70"></div>
      </div>
    {:else if error}
      <div class="p-8 text-sm text-red-300">{error}</div>
    {:else if !selected}
      <div class="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
        <span
          class="grid size-12 place-items-center rounded-lg border border-edge bg-ink-900/60 text-muted-foreground/65"
        >
          <Icon name="box" size={16} />
        </span>
        <p class="text-sm text-muted-foreground">No task selected</p>
        <Button href="/tasks/new" class="mt-1">
          <Icon name="plus" size={12} />
          Create a task
        </Button>
      </div>
    {:else}
      <div class="min-h-0 p-5">
        <PageHeader
          crumbs={[{ label: "Tasks", href: "/tasks" }, { label: selected.title }]}
          title={selected.title}
          subtitle={selected.goal}
          subtitleClass="line-clamp-2 max-w-3xl"
        >
          {#snippet actions()}
            <Button href={`/runs/${selected.latestRunId}`}>
              Open run
              <Icon name="arrow-up-right" size={12} />
            </Button>
          {/snippet}
        </PageHeader>

        <!-- Status + timing strip -->
        <div class="mt-4 flex flex-wrap items-center gap-x-5 gap-y-2">
          <StatusBadge status={selected.status} />
          <span
            class="flex items-center gap-1.5 font-mono text-2xs text-muted-foreground/65"
            title={formatDateTime(selected.createdAt)}
          >
            <Icon name="clock" size={12} />
            {relative(selected.createdAt)}
          </span>
          {#if tasks.length > 1}
            <span class="ml-auto flex items-center gap-1.5 text-2xs text-muted-foreground/65">
              <Icon name="layers" size={12} />
              {tasks.length} tasks in queue
            </span>
          {/if}
        </div>

        <!-- Command strip: 3 metrics across the full width (status lives in the strip above) -->
        <dl class="mt-4 grid grid-cols-2 gap-2 md:grid-cols-3">
          <div class="metric-chip">
            <dt>Mode</dt>
            <dd>agent</dd>
          </div>
          <div class="metric-chip">
            <dt>Run</dt>
            <dd>{selected.latestRunId.slice(0, 12)}</dd>
          </div>
          <div class="metric-chip">
            <dt>Queued</dt>
            <dd>{relative(selected.createdAt)}</dd>
          </div>
        </dl>

        <!-- Composite two-zone grid: objective + latest run + queue glance -->
        <div class="mt-4 grid gap-4 xl:grid-cols-[minmax(0,1fr)_300px]">
          <div class="min-w-0 space-y-4">
            <!-- Objective -->
            <section class="panel min-h-0 p-5">
              <div class="flex items-center gap-2">
                <span class="text-muted-foreground/65"><Icon name="spark" size={14} /></span>
                <h2 class="text-sm font-semibold tracking-tight text-foreground">Objective</h2>
              </div>
              <p
                class="mt-3 max-h-72 overflow-y-auto whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground agent-scroll"
              >
                {selected.goal}
              </p>
            </section>

            <!-- Latest run: live plan/budget snapshot without leaving the queue -->
            {#if runtime && runtime.plan.steps.length > 0}
              <section class="panel min-h-0 p-5">
                <div class="flex items-center gap-2">
                  <span class="text-muted-foreground/65"><Icon name="activity" size={14} /></span>
                  <h2 class="text-sm font-semibold tracking-tight text-foreground">Latest run</h2>
                  <a
                    href={`/runs/${selected.latestRunId}`}
                    class="ml-auto text-2xs text-mint-400 underline-offset-4 transition-colors hover:underline"
                  >
                    Details
                  </a>
                </div>
                <div class="mt-3 grid gap-3 lg:grid-cols-2">
                  <PlanPanel plan={runtime.plan} />
                  <BudgetPanel
                    budget={runtime.budget}
                    usage={runtime.usage}
                    workspaceRevision={runtime.workspaceRevision}
                  />
                </div>
                {#if runtime.report}
                  <p
                    class="mt-3 rounded-lg border border-edge bg-ink-800/50 p-3 text-sm leading-relaxed text-muted-foreground"
                  >
                    <span
                      class={`font-semibold ${runtime.report.outcome === "succeeded" ? "text-mint-400" : "text-red-300"}`}
                    >
                      {runtime.report.outcome}
                    </span>
                    — {runtime.report.summary}
                  </p>
                {/if}
              </section>
            {/if}
          </div>

          <!-- Queue glance: right rail with history + status counts (no bottom stack) -->
          <section class="panel min-h-0 p-5">
            <div class="flex items-center justify-between">
              <h2 class="text-sm font-semibold tracking-tight text-foreground">Queue glance</h2>
              <span class="font-mono text-2xs text-muted-foreground/65">{tasks.length} total</span>
            </div>
            <dl class="mt-3 grid grid-cols-3 gap-1.5">
              <div class="rounded-md border border-edge bg-ink-800/50 px-2 py-1.5 text-center">
                <dt
                  class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65"
                >
                  Running
                </dt>
                <dd class="mt-0.5 font-mono text-sm text-sky-300">{runningCount}</dd>
              </div>
              <div class="rounded-md border border-edge bg-ink-800/50 px-2 py-1.5 text-center">
                <dt
                  class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65"
                >
                  Done
                </dt>
                <dd class="mt-0.5 font-mono text-sm text-mint-400">{succeededCount}</dd>
              </div>
              <div class="rounded-md border border-edge bg-ink-800/50 px-2 py-1.5 text-center">
                <dt
                  class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65"
                >
                  Failed
                </dt>
                <dd class="mt-0.5 font-mono text-sm text-red-300">{failedCount}</dd>
              </div>
            </dl>

            <div class="mt-4 flex items-center justify-between">
              <h3
                class="text-2xs font-semibold uppercase tracking-[0.12em] text-muted-foreground/65"
              >
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
                          ? "bg-muted-foreground/65"
                          : "bg-muted-foreground/65"
                    }`}
                    aria-hidden="true"
                  ></span>
                  <a
                    href={`/runs/${task.latestRunId}`}
                    class="block truncate text-xs font-medium text-muted-foreground transition-colors hover:text-mint-400"
                  >
                    {task.title}
                  </a>
                  <p class="mt-0.5 flex items-center gap-1.5 text-2xs text-muted-foreground/65">
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
