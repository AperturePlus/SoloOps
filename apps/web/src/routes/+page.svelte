<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { api, ApiClientError } from "$lib/api";
  import type { TaskSummary } from "$lib/contracts";
  import { relativeTime } from "$lib/time";
  import Icon, { type IconName } from "$lib/components/Icon.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";

  let booting = $state(true);
  let authed = $state(false);
  let tasks = $state<TaskSummary[]>([]);
  let loading = $state(true);
  let error = $state("");

  onMount(async () => {
    try {
      await api("/api/auth/session");
      authed = true;
    } catch {
      authed = false;
    } finally {
      booting = false;
    }
    if (!authed) return;
    try {
      const response = await api<{ items: TaskSummary[] }>("/api/tasks");
      tasks = response.items;
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not load tasks";
    } finally {
      loading = false;
    }
  });

  const runningCount = $derived(tasks.filter((task) => task.status === "running").length);
  const succeededCount = $derived(tasks.filter((task) => task.status === "succeeded").length);
  const failedCount = $derived(tasks.filter((task) => task.status === "failed").length);

  const PIPELINE = [
    { label: "queued", state: "done" },
    { label: "planning", state: "done" },
    { label: "running", state: "current" },
    { label: "reporting", state: "idle" },
    { label: "succeeded", state: "idle" }
  ];

  const CAPABILITIES: { icon: IconName; title: string; text: string }[] = [
    {
      icon: "spark",
      title: "Plan + budget first",
      text: "Runs open with an explicit plan and model/tool budgets before anything executes."
    },
    {
      icon: "shield",
      title: "Approval-gated tools",
      text: "Workspace writes, processes and network calls pause for explicit owner review."
    },
    {
      icon: "eye",
      title: "Evidence & reports",
      text: "Every action leaves evidence; finished runs carry a replayable final report."
    },
    {
      icon: "command",
      title: "Durable queue",
      text: "Outcomes survive restarts. Leases, retries and recovery are handled for you."
    }
  ];

  function relative(ts: number): string {
    return relativeTime(ts);
  }
</script>

<svelte:head>
  <title>Overview · SoloOps</title>
</svelte:head>

<div class="ops-grid-bg agent-scroll h-screen overflow-y-auto">
  <!-- Top command bar -->
  <header
    class="sticky top-0 z-20 flex h-14 items-center gap-3 border-b border-edge bg-ink-950/80 px-6 backdrop-blur-xl"
  >
    <div class="flex items-center gap-2.5">
      <span
        class="grid size-8 place-items-center rounded-lg border border-mint-400/25 bg-mint-400/10 text-mint-400"
      >
        <Icon name="command" size={15} />
      </span>
      <div class="leading-tight">
        <p class="text-[13px] font-semibold tracking-tight text-slate-100">SoloOps</p>
        <p class="font-mono text-[10px] text-slate-600">private control plane</p>
      </div>
    </div>

    <div class="ml-auto flex items-center gap-2">
      {#if authed}
        <span class="hidden items-center gap-1.5 text-[11px] text-slate-400 sm:flex">
          <span class="size-1.5 rounded-full bg-mint-400 agent-live-ring"></span>
          signed in as owner
        </span>
        <a
          href="/tasks"
          class="flex items-center gap-1.5 rounded-md bg-mint-400 px-3 py-1.5 text-xs font-semibold text-ink-950 transition-opacity hover:opacity-90"
        >
          Open workspace
          <Icon name="arrow-up-right" size={12} />
        </a>
      {:else}
        <span class="text-[11px] text-slate-500">guest session</span>
        <a
          href="/login"
          class="flex items-center gap-1.5 rounded-md bg-mint-400 px-3 py-1.5 text-xs font-semibold text-ink-950 transition-opacity hover:opacity-90"
        >
          Sign in
          <Icon name="arrow-right" size={12} />
        </a>
      {/if}
    </div>
  </header>

  <!-- Composite body -->
  <main class="mx-auto grid max-w-[1400px] grid-cols-12 gap-5 p-6">
    <!-- Left zone: hero + pipeline + capability grid -->
    <div class="col-span-12 space-y-5 lg:col-span-8">
      <section class="panel p-6">
        <p class="text-[11px] font-medium uppercase tracking-[0.2em] text-mint-400">
          Agent operations
        </p>
        <h1 class="mt-2 max-w-xl text-[22px] font-semibold tracking-tight text-slate-100">
          A private execution queue for goal-driven agent work.
        </h1>
        <p class="mt-2 max-w-xl text-[13px] leading-relaxed text-slate-400">
          SoloOps plans, gates, measures and reports every run — on your machine, under your
          control. The workspace keeps the queue, live runs and settings in one dashboard.
        </p>

        <!-- Lifecycle pipeline strip -->
        <div class="mt-5 flex items-center gap-0 overflow-x-auto pb-1">
          {#each PIPELINE as stage, index (stage.label)}
            <div class="flow-step shrink-0" data-state={stage.state}>
              <span
                class={`size-1.5 shrink-0 rounded-full ${
                  stage.state === "done"
                    ? "bg-mint-400"
                    : stage.state === "current"
                      ? "bg-slate-200 agent-live-ring"
                      : "bg-slate-700"
                }`}
              ></span>
              {stage.label}
              {#if index < PIPELINE.length - 1}
                <span class="mx-2 h-px w-6 bg-white/10"></span>
              {/if}
            </div>
          {/each}
        </div>
      </section>

      <!-- Capability grid: 2×2, no stacking -->
      <div class="grid gap-4 sm:grid-cols-2">
        {#each CAPABILITIES as item (item.title)}
          <article class="panel panel-hover p-4">
            <div class="flex items-center gap-2.5">
              <span
                class="grid size-7 shrink-0 place-items-center rounded-md border border-white/10 bg-white/[0.03] text-slate-300"
              >
                <Icon name={item.icon} size={13} />
              </span>
              <h2 class="text-[12.5px] font-semibold tracking-tight text-slate-200">
                {item.title}
              </h2>
            </div>
            <p class="mt-2.5 text-[12px] leading-relaxed text-slate-500">{item.text}</p>
          </article>
        {/each}
      </div>
    </div>

    <!-- Right zone: queue glance (authed) or sign-in CTA (guest) -->
    <aside class="col-span-12 space-y-4 lg:col-span-4">
      {#if booting}
        <div class="space-y-3">
          <div class="h-48 animate-pulse rounded-xl bg-white/[0.04]"></div>
          <div class="h-64 animate-pulse rounded-xl bg-white/[0.04]"></div>
        </div>
      {:else if authed}
        <section class="panel p-5">
          <div class="flex items-center justify-between">
            <h2
              class="flex items-center gap-2 text-[13px] font-semibold tracking-tight text-slate-200"
            >
              <Icon name="layers" size={13} class="text-slate-500" />
              Queue glance
            </h2>
            <a
              href="/tasks"
              class="text-[11px] text-mint-400 transition-colors hover:text-mint-300"
            >
              View all
            </a>
          </div>

          <dl class="mt-3 grid grid-cols-3 gap-1.5">
            <div class="rounded-md border border-white/5 bg-white/[0.02] px-2 py-2 text-center">
              <dt class="text-[9px] font-medium uppercase tracking-[0.12em] text-slate-600">
                Live
              </dt>
              <dd class="mt-0.5 font-mono text-base text-sky-300">{runningCount}</dd>
            </div>
            <div class="rounded-md border border-white/5 bg-white/[0.02] px-2 py-2 text-center">
              <dt class="text-[9px] font-medium uppercase tracking-[0.12em] text-slate-600">
                Done
              </dt>
              <dd class="mt-0.5 font-mono text-base text-mint-300">{succeededCount}</dd>
            </div>
            <div class="rounded-md border border-white/5 bg-white/[0.02] px-2 py-2 text-center">
              <dt class="text-[9px] font-medium uppercase tracking-[0.12em] text-slate-600">
                Failed
              </dt>
              <dd class="mt-0.5 font-mono text-base text-red-300">{failedCount}</dd>
            </div>
          </dl>

          <div class="mt-4 flex items-center justify-between">
            <h3 class="text-[10.5px] font-semibold uppercase tracking-[0.12em] text-slate-500">
              Recent runs
            </h3>
          </div>
          {#if loading}
            <div class="mt-2 space-y-2">
              <div class="h-12 animate-pulse rounded-lg bg-white/[0.04]"></div>
              <div class="h-12 animate-pulse rounded-lg bg-white/[0.04]"></div>
            </div>
          {:else if error}
            <p class="mt-2 text-[12px] text-red-300">{error}</p>
          {:else if tasks.length === 0}
            <p class="mt-3 text-[12px] leading-relaxed text-slate-500">
              No runs yet. Create your first task to start the queue.
            </p>
          {:else}
            <ul class="agent-scroll mt-2 max-h-72 space-y-1 overflow-y-auto pr-1">
              {#each tasks.slice(0, 6) as task (task.id)}
                <li>
                  <a
                    href={`/runs/${task.latestRunId}`}
                    class="group flex items-center gap-2.5 rounded-lg border border-transparent px-2 py-1.5 transition-colors hover:border-white/5 hover:bg-white/[0.03]"
                  >
                    <span
                      class={`size-1.5 shrink-0 rounded-full ${
                        task.status === "succeeded"
                          ? "bg-mint-400"
                          : task.status === "failed"
                            ? "bg-red-400"
                            : "bg-sky-400"
                      }`}
                    ></span>
                    <span class="min-w-0 flex-1">
                      <span
                        class="block truncate text-[12.5px] text-slate-300 group-hover:text-slate-100"
                      >
                        {task.title}
                      </span>
                      <span class="mt-0.5 block font-mono text-[10px] text-slate-600">
                        {relative(task.createdAt)}
                      </span>
                    </span>
                    <StatusBadge status={task.status} />
                  </a>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      {:else}
        <section class="panel p-5">
          <div class="flex items-center gap-2.5">
            <span
              class="grid size-8 place-items-center rounded-lg border border-edge bg-ink-900/70 text-slate-300"
            >
              <Icon name="lock" size={14} />
            </span>
            <div>
              <p class="text-[13px] font-semibold tracking-tight text-slate-200">Owner access</p>
              <p class="text-[11px] text-slate-500">Restricted workspace</p>
            </div>
          </div>
          <p class="mt-4 text-[12.5px] leading-relaxed text-slate-400">
            The queue, live runs and owner settings are private. Sign in with the Owner account
            initialized from the local terminal.
          </p>
          <a
            href="/login"
            class="mt-4 flex items-center justify-center gap-1.5 rounded-md bg-mint-400 px-3 py-2 text-[13px] font-semibold text-ink-950 transition-opacity hover:opacity-90"
          >
            Sign in
            <Icon name="arrow-right" size={13} />
          </a>
          <div class="command-strip mt-4">
            <div class="px-3 py-2.5">
              <p class="text-[9.5px] font-medium uppercase tracking-[0.12em] text-slate-600">
                Mode
              </p>
              <p class="mt-0.5 font-mono text-[12.5px] text-slate-300">agent</p>
            </div>
            <div class="px-3 py-2.5">
              <p class="text-[9.5px] font-medium uppercase tracking-[0.12em] text-slate-600">
                Lease
              </p>
              <p class="mt-0.5 font-mono text-[12.5px] text-slate-300">owner-local</p>
            </div>
          </div>
        </section>
      {/if}
    </aside>
  </main>
</div>
