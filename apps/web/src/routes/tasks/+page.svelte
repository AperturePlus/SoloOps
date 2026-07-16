<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import type { TaskSummary } from "@soloops/contracts";
  import { api, ApiClientError } from "$lib/api";

  let tasks = $state<TaskSummary[]>([]);
  let loading = $state(true);
  let error = $state("");

  onMount(async () => {
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

  async function logout() {
    await api("/api/auth/logout", { method: "POST" });
    await goto("/login");
  }

  function statusClass(status: string) {
    if (status === "blocked" || status === "failed") return "border-amber-400/30 bg-amber-400/10 text-amber-300";
    if (status === "succeeded") return "border-mint-400/30 bg-mint-400/10 text-mint-400";
    return "border-sky-400/30 bg-sky-400/10 text-sky-300";
  }
</script>

<main>
  <div class="flex flex-wrap items-end justify-between gap-4">
    <div>
      <p class="text-sm font-medium uppercase tracking-[0.2em] text-mint-400">Execution queue</p>
      <h1 class="mt-2 text-4xl font-semibold tracking-tight">Tasks</h1>
    </div>
    <div class="flex gap-3">
      <button class="rounded-lg border border-white/10 px-4 py-2 text-sm text-slate-300 hover:bg-white/5" onclick={logout}>Sign out</button>
      <a class="rounded-lg bg-mint-400 px-4 py-2 text-sm font-semibold text-ink-950 hover:bg-mint-500" href="/tasks/new">New task</a>
    </div>
  </div>

  {#if loading}
    <p class="mt-10 text-slate-400">Loading tasks…</p>
  {:else if error}
    <p class="mt-10 text-red-300">{error}</p>
  {:else if tasks.length === 0}
    <div class="mt-10 rounded-2xl border border-dashed border-white/15 p-10 text-center">
      <p class="text-lg">No tasks yet</p>
      <p class="mt-2 text-sm text-slate-400">Create one to exercise the safe Phase 0 pipeline.</p>
    </div>
  {:else}
    <div class="mt-8 grid gap-3">
      {#each tasks as task (task.id)}
        <a class="group rounded-xl border border-white/10 bg-ink-900/70 p-5 transition hover:border-white/20 hover:bg-ink-800/70" href={`/runs/${task.latestRunId}`}>
          <div class="flex items-start justify-between gap-4">
            <div>
              <h2 class="font-medium group-hover:text-mint-400">{task.title}</h2>
              <p class="mt-1 line-clamp-2 text-sm text-slate-400">{task.goal}</p>
            </div>
            <span class={`rounded-full border px-2.5 py-1 text-xs ${statusClass(task.status)}`}>{task.status}</span>
          </div>
          <p class="mt-4 text-xs text-slate-500">{new Date(task.createdAt).toLocaleString()}</p>
        </a>
      {/each}
    </div>
  {/if}
</main>
