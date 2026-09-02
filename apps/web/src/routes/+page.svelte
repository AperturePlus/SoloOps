<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { api, ApiClientError } from "$lib/api";
  import type { TaskSummary } from "$lib/contracts";
  import { relativeTime } from "$lib/time";
  import Icon, { type IconName } from "$lib/components/Icon.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import { Button } from "$lib/components/ui/button";

  let booting = $state(true);
  let authed = $state(false);
  let tasks = $state<TaskSummary[]>([]);
  let loading = $state(true);
  let error = $state("");

  const TERMINAL = new Set(["succeeded", "failed", "cancelled"]);

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

  const activeTasks = $derived(tasks.filter((task) => !TERMINAL.has(task.status)));
  const awaitingTasks = $derived(tasks.filter((task) => task.status === "waiting_for_approval"));
  const succeededCount = $derived(tasks.filter((task) => task.status === "succeeded").length);
  const failedCount = $derived(tasks.filter((task) => task.status === "failed").length);
  const recentTasks = $derived(tasks.slice(0, 8));

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

  function statusDot(status: TaskSummary["status"]): string {
    if (status === "succeeded") return "bg-mint-400";
    if (status === "failed" || status === "cancelled") return "bg-red-400";
    if (status === "waiting_for_approval") return "bg-amber-300";
    return "bg-sky-400";
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
        <Icon name="command" size={14} />
      </span>
      <div class="leading-tight">
        <p class="text-sm font-semibold tracking-tight text-foreground">SoloOps</p>
        <p class="font-mono text-2xs text-muted-foreground/65">private control plane</p>
      </div>
    </div>

    <div class="ml-auto flex items-center gap-2">
      {#if authed}
        <span class="hidden items-center gap-1.5 text-2xs text-muted-foreground sm:flex">
          <span class="size-1.5 rounded-full bg-mint-400 agent-live-ring"></span>
          signed in as owner
        </span>
        <Button href="/tasks/new">
          <Icon name="plus" size={12} />
          New task
        </Button>
      {:else}
        <span class="text-2xs text-muted-foreground/65">guest session</span>
        <Button href="/login">
          Sign in
          <Icon name="arrow-right" size={12} />
        </Button>
      {/if}
    </div>
  </header>

  <main class="mx-auto max-w-7xl p-6">
    {#if booting}
      <div class="grid grid-cols-2 gap-3 sm:grid-cols-4" aria-hidden="true">
        {#each [0, 1, 2, 3] as i (i)}
          <div class="h-20 animate-pulse rounded-lg bg-ink-800/70"></div>
        {/each}
      </div>
    {:else if authed}
      <!-- Stat strip -->
      <div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <a href="/tasks" class="panel panel-hover flex items-center gap-3 p-4">
          <span
            class="grid size-9 shrink-0 place-items-center rounded-lg bg-sky-400/10 text-sky-300"
          >
            <Icon name="activity" size={14} />
          </span>
          <span class="min-w-0">
            <span
              class="block text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65"
            >
              Active runs
            </span>
            <span class="mt-0.5 block font-mono text-2xl tabular-nums text-foreground">
              {activeTasks.length}
            </span>
          </span>
        </a>
        <a
          href={awaitingTasks[0] ? `/runs/${awaitingTasks[0].latestRunId}` : "/tasks"}
          class={`panel flex items-center gap-3 p-4 transition-colors ${
            awaitingTasks.length > 0
              ? "border-amber-400/40 bg-amber-400/[0.06] hover:bg-amber-400/10"
              : "panel-hover"
          }`}
        >
          <span
            class={`grid size-9 shrink-0 place-items-center rounded-lg ${awaitingTasks.length > 0 ? "bg-amber-400/15 text-amber-300" : "bg-ink-800/60 text-muted-foreground"}`}
          >
            <Icon name="shield" size={14} />
          </span>
          <span class="min-w-0">
            <span
              class="block text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65"
            >
              Awaiting approval
            </span>
            <span
              class={`mt-0.5 block font-mono text-2xl tabular-nums ${awaitingTasks.length > 0 ? "text-amber-200" : "text-foreground"}`}
            >
              {awaitingTasks.length}
            </span>
          </span>
        </a>
        <a href="/tasks" class="panel panel-hover flex items-center gap-3 p-4">
          <span
            class="grid size-9 shrink-0 place-items-center rounded-lg bg-mint-400/10 text-mint-400"
          >
            <Icon name="check" size={14} />
          </span>
          <span class="min-w-0">
            <span
              class="block text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65"
            >
              Succeeded
            </span>
            <span class="mt-0.5 block font-mono text-2xl tabular-nums text-foreground">
              {succeededCount}
            </span>
          </span>
        </a>
        <a href="/tasks" class="panel panel-hover flex items-center gap-3 p-4">
          <span
            class="grid size-9 shrink-0 place-items-center rounded-lg bg-red-400/10 text-red-300"
          >
            <Icon name="x" size={14} />
          </span>
          <span class="min-w-0">
            <span
              class="block text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65"
            >
              Failed
            </span>
            <span class="mt-0.5 block font-mono text-2xl tabular-nums text-foreground">
              {failedCount}
            </span>
          </span>
        </a>
      </div>

      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <!-- Recent tasks queue -->
        <section class="panel col-span-12 p-5 lg:col-span-8">
          <div class="flex items-center justify-between gap-3">
            <h2
              class="flex items-center gap-2 text-sm font-semibold tracking-tight text-foreground"
            >
              <Icon name="layers" size={14} class="text-muted-foreground/65" />
              Recent tasks
              {#if tasks.length > 0}
                <span class="font-mono text-2xs font-normal text-muted-foreground/65"
                  >{tasks.length}</span
                >
              {/if}
            </h2>
            {#if tasks.length > 0}
              <a
                href="/tasks"
                class="text-2xs text-mint-400 underline-offset-4 transition-colors hover:underline"
              >
                View all
              </a>
            {/if}
          </div>

          {#if loading}
            <div class="mt-4 space-y-2">
              <div class="h-12 animate-pulse rounded-lg bg-ink-800/70"></div>
              <div class="h-12 animate-pulse rounded-lg bg-ink-800/70"></div>
              <div class="h-12 animate-pulse rounded-lg bg-ink-800/70"></div>
            </div>
          {:else if error}
            <p class="mt-4 text-sm text-red-300">{error}</p>
          {:else if tasks.length === 0}
            <!-- Empty state doubles as the product introduction -->
            <div class="mt-4 rounded-lg border border-dashed border-edge p-6">
              <p class="text-sm font-semibold text-foreground">No runs yet</p>
              <p class="mt-1 max-w-md text-sm leading-relaxed text-muted-foreground/65">
                Create your first task — SoloOps will plan the work, gate every privileged action
                through you, and deliver a verified report.
              </p>
              <Button href="/tasks/new">
                <Icon name="plus" size={14} />
                Create your first task
              </Button>

              <ul class="mt-6 grid gap-x-6 gap-y-4 sm:grid-cols-2">
                {#each CAPABILITIES as item (item.title)}
                  <li class="flex gap-2.5">
                    <span
                      class="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md border border-edge bg-ink-800/60 text-muted-foreground"
                    >
                      <Icon name={item.icon} size={12} />
                    </span>
                    <div class="min-w-0">
                      <p class="text-sm font-medium text-muted-foreground">{item.title}</p>
                      <p class="mt-0.5 text-xs leading-relaxed text-muted-foreground/65">
                        {item.text}
                      </p>
                    </div>
                  </li>
                {/each}
              </ul>
            </div>
          {:else}
            <ul class="mt-3 divide-y divide-edge">
              {#each recentTasks as task (task.id)}
                <li>
                  <a
                    href={`/runs/${task.latestRunId}`}
                    class="group flex items-center gap-3 rounded-lg px-2 py-2.5 transition-colors duration-200 hover:bg-ink-800/60"
                  >
                    <span class={`size-1.5 shrink-0 rounded-full ${statusDot(task.status)}`}></span>
                    <span class="min-w-0 flex-1">
                      <span
                        class="block truncate text-sm text-foreground group-hover:text-foreground"
                      >
                        {task.title}
                      </span>
                      <span class="mt-0.5 block font-mono text-2xs text-muted-foreground/65">
                        {relativeTime(task.createdAt)}
                      </span>
                    </span>
                    <StatusBadge status={task.status} />
                    <Icon
                      name="chevron-right"
                      size={14}
                      class="shrink-0 text-muted-foreground/65 transition-colors group-hover:text-muted-foreground"
                    />
                  </a>
                </li>
              {/each}
            </ul>
          {/if}
        </section>

        <!-- Right rail: approvals · quick actions · settings -->
        <aside class="col-span-12 space-y-4 lg:col-span-4">
          {#if awaitingTasks.length > 0}
            <section
              class="animate-fade-up rounded-lg border border-amber-400/40 bg-amber-400/[0.07] p-4"
              aria-label="Approvals pending"
            >
              <div class="flex items-center gap-2">
                <span class="agent-live-ring size-1.5 rounded-full bg-amber-300"></span>
                <h2 class="text-sm font-semibold tracking-tight text-amber-100">
                  {awaitingTasks.length}
                  {awaitingTasks.length === 1 ? "run awaits" : "runs await"} your approval
                </h2>
              </div>
              <ul class="mt-2.5 space-y-1">
                {#each awaitingTasks.slice(0, 3) as task (task.id)}
                  <li>
                    <a
                      href={`/runs/${task.latestRunId}`}
                      class="flex items-center gap-2 rounded-lg px-1.5 py-1.5 text-sm text-amber-100/90 transition-colors hover:bg-amber-400/10"
                    >
                      <span class="min-w-0 flex-1 truncate">{task.title}</span>
                      <Icon name="arrow-right" size={12} class="shrink-0" />
                    </a>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          <section class="panel p-4">
            <h2 class="text-2xs font-semibold uppercase tracking-[0.12em] text-muted-foreground/65">
              Quick actions
            </h2>
            <div class="mt-3 space-y-1.5">
              <a
                href="/tasks/new"
                class="flex items-center gap-2.5 rounded-lg border border-edge px-3 py-2.5 text-sm text-foreground transition-colors hover:border-mint-400/40 hover:bg-mint-400/[0.06] hover:text-mint-400"
              >
                <Icon name="plus" size={14} class="text-muted-foreground/65" />
                New task
              </a>
              <a
                href="/tasks"
                class="flex items-center gap-2.5 rounded-lg border border-edge px-3 py-2.5 text-sm text-foreground transition-colors hover:border-edge-strong hover:bg-ink-800/60"
              >
                <Icon name="list" size={14} class="text-muted-foreground/65" />
                Open task queue
              </a>
            </div>
          </section>

          <section class="panel p-4">
            <h2 class="text-2xs font-semibold uppercase tracking-[0.12em] text-muted-foreground/65">
              Owner settings
            </h2>
            <a
              href="/settings/notifications"
              class="group mt-3 flex items-center gap-2.5 rounded-lg px-1.5 py-2 transition-colors hover:bg-ink-800/60"
            >
              <span
                class="grid size-7 shrink-0 place-items-center rounded-md border border-edge bg-ink-800/60 text-muted-foreground"
              >
                <Icon name="mail" size={12} />
              </span>
              <span class="min-w-0 flex-1">
                <span class="block text-sm text-muted-foreground group-hover:text-foreground">
                  Public IP notifications
                </span>
                <span class="mt-0.5 block text-2xs text-muted-foreground/65"
                  >Alert on IPv4 change</span
                >
              </span>
              <Icon
                name="chevron-right"
                size={14}
                class="shrink-0 text-muted-foreground/65 transition-colors group-hover:text-muted-foreground"
              />
            </a>
            <div class="command-strip mt-3">
              <div class="px-3 py-2">
                <p
                  class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65"
                >
                  Mode
                </p>
                <p class="mt-0.5 font-mono text-sm text-muted-foreground">agent</p>
              </div>
              <div class="px-3 py-2">
                <p
                  class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65"
                >
                  Lease
                </p>
                <p class="mt-0.5 font-mono text-sm text-muted-foreground">owner-local</p>
              </div>
            </div>
          </section>
        </aside>
      </div>
    {:else}
      <!-- Guest: compact hero + sign-in -->
      <div class="mx-auto mt-10 max-w-2xl text-center">
        <p class="text-2xs font-medium uppercase tracking-[0.2em] text-mint-400">
          Agent operations
        </p>
        <h1 class="mt-2 text-2xl font-semibold tracking-tight text-foreground">
          A private execution queue for goal-driven agent work.
        </h1>
        <p class="mx-auto mt-2 max-w-lg text-sm leading-relaxed text-muted-foreground">
          SoloOps plans, gates, measures and reports every run — on your machine, under your
          control.
        </p>
        <Button href="/login" size="lg">
          Sign in as owner
          <Icon name="arrow-right" size={14} />
        </Button>

        <ul class="mx-auto mt-10 grid max-w-xl gap-x-6 gap-y-5 text-left sm:grid-cols-2">
          {#each CAPABILITIES as item (item.title)}
            <li class="flex gap-3">
              <span
                class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border border-edge bg-ink-800/60 text-muted-foreground"
              >
                <Icon name={item.icon} size={12} />
              </span>
              <div class="min-w-0">
                <p class="text-sm font-semibold text-foreground">{item.title}</p>
                <p class="mt-0.5 text-xs leading-relaxed text-muted-foreground/65">{item.text}</p>
              </div>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </main>
</div>
