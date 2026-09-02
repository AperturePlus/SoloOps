<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import type { EventEnvelope, RunDetail, RuntimeSnapshot, TaskSummary } from "$lib/contracts";
  import { api, ApiClientError } from "$lib/api";
  import { formatDuration } from "$lib/time";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import WorkingIndicator from "$lib/components/WorkingIndicator.svelte";
  import PlanPanel from "$lib/components/PlanPanel.svelte";
  import BudgetPanel from "$lib/components/BudgetPanel.svelte";
  import ToolCallCard from "$lib/components/ToolCallCard.svelte";
  import EvidenceList from "$lib/components/EvidenceList.svelte";
  import ReportCard from "$lib/components/ReportCard.svelte";
  import EventStream from "$lib/components/EventStream.svelte";
  import ApprovalBanner from "$lib/components/ApprovalBanner.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { Button } from "$lib/components/ui/button";

  let { data } = $props();
  let run = $state<RunDetail | null>(null);
  let task = $state<TaskSummary | null>(null);
  let events = $state<EventEnvelope[]>([]);
  let runtime = $state<RuntimeSnapshot | null>(null);
  let deciding = $state(false);
  let error = $state("");
  let socket: WebSocket | undefined;

  const EVENT_PAGE_SIZE = 200;
  const MAX_RECONNECT_DELAY_MS = 30_000;
  const TERMINAL_STATUSES = new Set(["succeeded", "failed", "cancelled"]);

  let now = $state(Date.now());
  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), 1_000);
    return () => clearInterval(timer);
  });

  const isActive = $derived(!!run && !TERMINAL_STATUSES.has(run.status));
  const startedAt = $derived(run?.startedAt ?? run?.createdAt ?? null);
  const elapsed = $derived(startedAt ? formatDuration((run?.finishedAt ?? now) - startedAt) : "");
  const toolNames = $derived(
    Object.fromEntries((runtime?.toolCalls ?? []).map((call) => [call.callId, call.name]))
  );
  const awaitingCalls = $derived(
    (runtime?.toolCalls ?? []).filter((call) => call.status === "waiting_for_approval")
  );
  const shortRunId = $derived(data.runId.length > 10 ? data.runId.slice(0, 10) : data.runId);

  // --- Activity column auto-scroll: pinned to newest unless user scrolls up ---
  let activityEl = $state<HTMLDivElement | undefined>();
  let pinned = $state(true);

  function onActivityScroll() {
    if (!activityEl) return;
    pinned = activityEl.scrollTop + activityEl.clientHeight >= activityEl.scrollHeight - 120;
  }

  function scrollActivity(behavior: "auto" | "smooth") {
    activityEl?.scrollTo({ top: activityEl.scrollHeight, behavior });
  }

  const contentVersion = $derived(
    events.length * 1_000 +
      (runtime?.toolCalls.length ?? 0) * 10 +
      (runtime?.plan.steps.length ?? 0) +
      (runtime?.evidence.length ?? 0) +
      (runtime?.report ? 1 : 0)
  );

  $effect(() => {
    void contentVersion;
    if (pinned) scrollActivity("smooth");
  });

  async function refreshRuntime() {
    try {
      runtime = await api<RuntimeSnapshot>(`/api/runs/${encodeURIComponent(data.runId)}/runtime`);
    } catch (cause) {
      if (!(cause instanceof ApiClientError && cause.status === 404)) throw cause;
    }
  }

  async function decide(callId: string, decision: "approve" | "deny") {
    deciding = true;
    try {
      await api(
        `/api/runs/${encodeURIComponent(data.runId)}/tool-calls/${encodeURIComponent(callId)}/decision`,
        {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ decision })
        }
      );
      await refreshRuntime();
    } finally {
      deciding = false;
    }
  }

  async function cancelRun() {
    run = await api<RunDetail>(`/api/runs/${encodeURIComponent(data.runId)}/cancel`, {
      method: "POST"
    });
  }

  function applyEvent(event: EventEnvelope) {
    if (events.some((item) => item.sequence === event.sequence)) return;
    events = [...events, event].sort((a, b) => a.sequence - b.sequence);
    if (event.type === "run.status_changed" && run) {
      run = {
        ...run,
        status: event.payload.to as RunDetail["status"],
        statusReason: (event.payload.reason as string | null | undefined) ?? run.statusReason
      };
    }
    if (
      event.type === "run.status_changed" ||
      event.type.startsWith("agent.") ||
      event.type.startsWith("tool.") ||
      event.type === "run.reported"
    ) {
      void refreshRuntime();
    }
  }

  onMount(() => {
    let stopped = false;
    let reconnectAttempt = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;

    function lastSequence() {
      return events.at(-1)?.sequence ?? 0;
    }

    async function fetchPersistedEvents() {
      let after = lastSequence();
      while (!stopped) {
        const backlog = await api<{ items: EventEnvelope[] }>(
          `/api/events?after=${after}&runId=${encodeURIComponent(data.runId)}`
        );
        backlog.items.forEach(applyEvent);
        if (backlog.items.length < EVENT_PAGE_SIZE) return;

        const nextAfter = lastSequence();
        if (nextAfter <= after) return;
        after = nextAfter;
      }
    }

    function scheduleReconnect(message = "Live updates disconnected. Reconnecting…") {
      if (stopped || reconnectTimer !== undefined) return;
      if (run && TERMINAL_STATUSES.has(run.status)) {
        stopped = true;
        error = "";
        return;
      }
      const delay = Math.min(1_000 * 2 ** reconnectAttempt, MAX_RECONNECT_DELAY_MS);
      reconnectAttempt += 1;
      error = message;
      reconnectTimer = setTimeout(() => {
        reconnectTimer = undefined;
        void connect();
      }, delay);
    }

    async function connect() {
      try {
        if (!run) {
          run = await api<RunDetail>(`/api/runs/${encodeURIComponent(data.runId)}`);
        }
        if (!task && run.taskId) {
          try {
            task = await api<TaskSummary>(`/api/tasks/${encodeURIComponent(run.taskId)}`);
          } catch {
            task = null;
          }
        }
        await refreshRuntime();
        await fetchPersistedEvents();
        if (stopped) return;

        const protocol = location.protocol === "https:" ? "wss:" : "ws:";
        const query = new URLSearchParams({
          after: String(lastSequence()),
          runId: data.runId
        });
        const nextSocket = new WebSocket(
          `${protocol}//${location.host}/api/events?${query.toString()}`
        );
        socket = nextSocket;
        nextSocket.onopen = () => {
          if (socket !== nextSocket) return;
          reconnectAttempt = 0;
          error = "";
        };
        nextSocket.onmessage = (message) => {
          try {
            applyEvent(JSON.parse(message.data));
          } catch {
            error = "Received an invalid live event. Reconnecting…";
            nextSocket.close();
          }
        };
        nextSocket.onerror = () => nextSocket.close();
        nextSocket.onclose = (event) => {
          if (socket !== nextSocket) return;
          socket = undefined;
          if (event.code === 4002) {
            stopped = true;
            error = "The persisted event stream is corrupt and requires operator repair.";
            return;
          }
          scheduleReconnect();
        };
      } catch (cause) {
        if (cause instanceof ApiClientError && cause.status === 401) {
          stopped = true;
          return goto("/login");
        }
        if (cause instanceof ApiClientError && cause.code === "event_stream_corrupt") {
          stopped = true;
          error = cause.message;
          return;
        }
        const message = cause instanceof Error ? cause.message : "Could not restore live updates";
        scheduleReconnect(`${message} Retrying…`);
      }
    }

    void connect();
    return () => {
      stopped = true;
      if (reconnectTimer !== undefined) clearTimeout(reconnectTimer);
      socket?.close();
      socket = undefined;
    };
  });
</script>

<svelte:head>
  <title>{task?.title ?? `Run ${shortRunId}`} · SoloOps</title>
</svelte:head>

<!-- ============ Top toolbar (sticky, ui-spec.md §2 run-page exception) ============ -->
<header
  class="sticky top-0 z-20 flex h-14 items-center gap-3 border-b border-edge bg-ink-950/80 px-6 backdrop-blur-xl"
>
  <a
    class="grid size-8 shrink-0 place-items-center rounded-lg border border-edge text-muted-foreground transition-all duration-200 hover:border-edge-strong hover:text-foreground"
    href="/tasks"
    aria-label="Back to tasks"
  >
    <Icon name="arrow-left" size={14} />
  </a>

  <div class="min-w-0 flex-1">
    <div class="flex items-center gap-2">
      <h1 class="truncate text-sm font-semibold tracking-tight text-foreground">
        {task?.title ?? "Run"}
      </h1>
      {#if runtime}
        <span class="hidden items-center gap-1 font-mono text-2xs text-muted-foreground/65 sm:flex">
          <Icon name="layers" size={12} />
          {runtime.checkpoint.replace(/_/g, " ")}
        </span>
      {/if}
    </div>
    <p class="truncate font-mono text-2xs text-muted-foreground/65" title={data.runId}>
      {data.runId}
    </p>
  </div>

  <div class="flex shrink-0 items-center gap-2.5">
    {#if elapsed}
      <span
        class="hidden items-center gap-1.5 font-mono text-2xs tabular-nums text-muted-foreground md:flex"
      >
        <Icon name="clock" size={12} />
        {elapsed}
      </span>
    {/if}
    {#if isActive}
      <span class="hidden items-center gap-1.5 text-2xs text-muted-foreground md:flex">
        <span
          class={`size-1.5 rounded-full ${error ? "animate-pulse bg-amber-300" : "agent-live-ring bg-mint-400"}`}
        ></span>
        {error ? "reconnecting" : "live"}
      </span>
    {/if}
    {#if run}
      <StatusBadge status={run.status} />
      {#if !TERMINAL_STATUSES.has(run.status)}
        <Button variant="destructive" size="sm" onclick={cancelRun}>Cancel</Button>
      {/if}
    {/if}
  </div>
</header>

<main class="mx-auto max-w-7xl px-6 py-5">
  {#if run?.statusReason || error}
    <div class="mb-4 space-y-2">
      {#if run?.statusReason}
        <div
          class="animate-fade-up rounded-lg border border-amber-400/20 bg-amber-400/[0.06] p-3 text-sm leading-relaxed text-amber-100"
        >
          {run.statusReason}
        </div>
      {/if}
      {#if error}
        <div
          class="animate-fade-up flex items-center gap-2 rounded-lg border border-red-400/20 bg-red-400/[0.06] p-3 text-sm text-red-200"
        >
          <span class="size-1.5 shrink-0 animate-pulse rounded-full bg-red-300"></span>
          {error}
        </div>
      {/if}
    </div>
  {/if}

  {#if !run && !error}
    <div class="grid grid-cols-12 gap-4" aria-hidden="true">
      <div class="col-span-12 space-y-4 lg:col-span-4">
        <div class="h-40 animate-pulse rounded-lg bg-ink-800/70"></div>
        <div class="h-28 animate-pulse rounded-lg bg-ink-800/70"></div>
        <div class="h-40 animate-pulse rounded-lg bg-ink-800/70"></div>
      </div>
      <div class="col-span-12 space-y-4 lg:col-span-8">
        <div class="h-40 animate-pulse rounded-lg bg-ink-800/70"></div>
        <div class="h-72 animate-pulse rounded-lg bg-ink-800/70"></div>
      </div>
    </div>
  {:else}
    {#if runtime && awaitingCalls.length}
      <div class="mb-4">
        <ApprovalBanner
          calls={awaitingCalls}
          busy={deciding}
          onDecide={(call, decision) => decide(call.callId, decision)}
        />
      </div>
    {/if}

    <!-- ============ Composite grid: plan rail · activity · report rail ============ -->
    <div class="grid grid-cols-12 gap-4">
      <!-- ---- Left rail: plan · budget · working · tool calls ---- -->
      <div class="col-span-12 space-y-4 lg:col-span-4 xl:col-span-3">
        {#if runtime}
          <div class="animate-fade-up">
            <PlanPanel plan={runtime.plan} />
          </div>

          <div class="animate-fade-up" style="animation-delay:0.08s">
            <BudgetPanel
              budget={runtime.budget}
              usage={runtime.usage}
              workspaceRevision={runtime.workspaceRevision}
            />
          </div>

          {#if isActive}
            <div class="animate-fade-up" style="animation-delay:0.12s">
              <WorkingIndicator checkpoint={runtime.checkpoint} />
            </div>
          {/if}

          {#if runtime.toolCalls.length}
            <section
              class="panel animate-fade-up p-3"
              style="animation-delay:0.16s"
              aria-label="Tool calls"
            >
              <div class="flex items-center justify-between px-1 pb-2">
                <h2 class="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                  Tool calls
                </h2>
                <span class="font-mono text-2xs text-muted-foreground/65"
                  >{runtime.toolCalls.length}</span
                >
              </div>
              <div class="agent-scroll max-h-80 space-y-1.5 overflow-y-auto pr-1">
                {#each runtime.toolCalls as call (call.callId)}
                  <ToolCallCard
                    {call}
                    onDecide={(target, decision) => decide(target.callId, decision)}
                  />
                {/each}
              </div>
            </section>
          {/if}
        {/if}
      </div>

      <!-- ---- Middle: activity stream (fixed-height internal scroll, fits viewport) ---- -->
      <div class="col-span-12 lg:col-span-8 xl:col-span-6">
        <section
          class="panel animate-fade-up flex h-full min-h-96 flex-col overflow-hidden"
          style="animation-delay:0.1s"
        >
          <div class="hairline flex h-10 shrink-0 items-center gap-2 px-3">
            <Icon name="activity" size={14} class="text-muted-foreground/65" />
            <h2 class="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
              Activity
            </h2>
            <span class="font-mono text-2xs text-muted-foreground/65">{events.length}</span>
            <span class="ml-auto flex items-center gap-2">
              {#if !pinned}
                <button
                  type="button"
                  class="flex items-center gap-1 rounded-md border border-edge px-2 py-1 text-2xs text-muted-foreground transition-colors hover:border-edge-strong"
                  onclick={() => {
                    pinned = true;
                    scrollActivity("smooth");
                  }}
                >
                  Jump to latest
                  <Icon name="chevron-down" size={12} />
                </button>
              {/if}
            </span>
          </div>

          <div
            bind:this={activityEl}
            onscroll={onActivityScroll}
            class="agent-scroll min-h-0 flex-1 overflow-y-auto px-2 py-2"
            style="max-height: calc(100vh - 240px)"
          >
            {#if events.length}
              <EventStream {events} {toolNames} />
            {:else}
              <p class="px-3 py-8 text-center text-sm text-muted-foreground/65">
                No activity yet — waiting for the run to start.
              </p>
            {/if}
          </div>
        </section>
      </div>

      <!-- ---- Right rail: report + evidence (stacked narrow rail, no horizontal squeeze) ---- -->
      <div
        class="col-span-12 space-y-4 lg:col-span-8 lg:row-start-2 xl:col-span-3 xl:row-start-auto"
      >
        {#if runtime}
          {#if runtime.report || runtime.evidence.length}
            {#if runtime.report}
              <div class="animate-fade-up" style="animation-delay:0.14s">
                <ReportCard report={runtime.report} />
              </div>
            {/if}
            {#if runtime.evidence.length}
              <div class="animate-fade-up" style="animation-delay:0.18s">
                <EvidenceList items={runtime.evidence} />
              </div>
            {/if}
          {:else}
            <section class="panel animate-fade-up p-5" style="animation-delay:0.14s">
              <div class="flex items-center gap-2">
                <Icon name="file" size={14} class="text-muted-foreground/65" />
                <h2 class="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                  Report
                </h2>
              </div>
              <p class="mt-3 text-sm leading-relaxed text-muted-foreground/65">
                No report or evidence yet — it appears as soon as the agent produces it.
              </p>
            </section>
          {/if}
        {/if}
      </div>
    </div>
  {/if}
</main>
