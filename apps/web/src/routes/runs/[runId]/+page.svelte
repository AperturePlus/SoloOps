<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import type { EventEnvelope, RunDetail } from "@soloops/contracts";
  import { api, ApiClientError } from "$lib/api";

  let { data } = $props();
  let run = $state<RunDetail | null>(null);
  let events = $state<EventEnvelope[]>([]);
  let error = $state("");
  let socket: WebSocket | undefined;

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
  }

  onMount(() => {
    async function initialize() {
      try {
        run = await api<RunDetail>(`/api/runs/${data.runId}`);
        const backlog = await api<{ items: EventEnvelope[] }>(`/api/events?after=0&runId=${data.runId}`);
        backlog.items.forEach(applyEvent);
        const after = events.at(-1)?.sequence ?? 0;
        const protocol = location.protocol === "https:" ? "wss:" : "ws:";
        socket = new WebSocket(`${protocol}//${location.host}/api/events?after=${after}&runId=${data.runId}`);
        socket.onmessage = (message) => applyEvent(JSON.parse(message.data));
        socket.onerror = () => { error = "Live updates disconnected. Reload to fetch persisted events."; };
      } catch (cause) {
        if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
        error = cause instanceof Error ? cause.message : "Could not load run";
      }
    }
    void initialize();
    return () => socket?.close();
  });
</script>

<main>
  <a class="text-sm text-slate-400 hover:text-white" href="/tasks">← Tasks</a>
  <div class="mt-5 flex flex-wrap items-start justify-between gap-4">
    <div>
      <p class="font-mono text-xs text-slate-500">{data.runId}</p>
      <h1 class="mt-2 text-4xl font-semibold tracking-tight">Run timeline</h1>
    </div>
    {#if run}<span class="rounded-full border border-amber-400/30 bg-amber-400/10 px-3 py-1 text-sm text-amber-300">{run.status}</span>{/if}
  </div>

  {#if run?.statusReason}
    <div class="mt-6 rounded-xl border border-amber-400/20 bg-amber-400/5 p-4 text-sm text-amber-100">{run.statusReason}</div>
  {/if}
  {#if error}<p class="mt-5 text-sm text-red-300">{error}</p>{/if}

  <ol class="relative mt-10 space-y-5 border-l border-white/10 pl-7">
    {#each events as event (event.sequence)}
      <li class="relative rounded-xl border border-white/10 bg-ink-900/70 p-4">
        <span class="absolute -left-[2.15rem] top-5 size-3 rounded-full border-2 border-ink-950 bg-mint-400"></span>
        <div class="flex flex-wrap items-center justify-between gap-2">
          <strong class="text-sm font-medium">{event.type}</strong>
          <time class="text-xs text-slate-500">{new Date(event.createdAt).toLocaleString()}</time>
        </div>
        <pre class="mt-3 overflow-auto text-xs text-slate-400">{JSON.stringify(event.payload, null, 2)}</pre>
      </li>
    {/each}
  </ol>
</main>
