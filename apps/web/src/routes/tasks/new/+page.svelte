<script lang="ts">
  import { goto } from "$app/navigation";
  import type { TaskSummary } from "@soloops/contracts";
  import { api, ApiClientError } from "$lib/api";

  let title = $state("");
  let goal = $state("");
  let error = $state("");
  let submitting = $state(false);

  async function createTask(event: SubmitEvent) {
    event.preventDefault();
    error = "";
    submitting = true;
    try {
      const task = await api<TaskSummary>("/api/tasks", { method: "POST", body: JSON.stringify({ title, goal }) });
      await goto(`/runs/${task.latestRunId}`);
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not create task";
    } finally {
      submitting = false;
    }
  }
</script>

<main class="mx-auto max-w-2xl">
  <a class="text-sm text-slate-400 hover:text-white" href="/tasks">← Tasks</a>
  <h1 class="mt-5 text-4xl font-semibold tracking-tight">Create a task</h1>
  <p class="mt-3 text-slate-400">Phase 0 persists and plans the task, then safely blocks before any real action.</p>
  <form class="mt-8 space-y-5" onsubmit={createTask}>
    <label class="block">
      <span class="mb-2 block text-sm text-slate-300">Title</span>
      <input class="w-full rounded-lg border border-white/10 bg-ink-900 px-3 py-2.5 outline-none focus:border-mint-400" bind:value={title} maxlength="160" required />
    </label>
    <label class="block">
      <span class="mb-2 block text-sm text-slate-300">Goal</span>
      <textarea class="min-h-48 w-full resize-y rounded-lg border border-white/10 bg-ink-900 px-3 py-2.5 outline-none focus:border-mint-400" bind:value={goal} maxlength="20000" required></textarea>
    </label>
    {#if error}<p class="text-sm text-red-300">{error}</p>{/if}
    <button class="rounded-lg bg-mint-400 px-5 py-2.5 font-semibold text-ink-950 hover:bg-mint-500 disabled:opacity-50" disabled={submitting}>
      {submitting ? "Creating…" : "Create task"}
    </button>
  </form>
</main>

