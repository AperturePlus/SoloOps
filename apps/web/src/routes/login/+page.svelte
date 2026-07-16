<script lang="ts">
  import { goto } from "$app/navigation";
  import { api } from "$lib/api";

  let username = $state("owner");
  let password = $state("");
  let error = $state("");
  let submitting = $state(false);

  async function login(event: SubmitEvent) {
    event.preventDefault();
    error = "";
    submitting = true;
    try {
      await api("/api/auth/login", { method: "POST", body: JSON.stringify({ username, password }) });
      await goto("/tasks");
    } catch (cause) {
      error = cause instanceof Error ? cause.message : "Login failed";
    } finally {
      submitting = false;
    }
  }
</script>

<main class="mx-auto max-w-md pt-10">
  <p class="mb-2 text-sm font-medium uppercase tracking-[0.24em] text-mint-400">Private control plane</p>
  <h1 class="text-4xl font-semibold tracking-tight">Owner sign in</h1>
  <p class="mt-3 text-slate-400">Use the Owner account initialized from the local terminal.</p>

  <form class="mt-8 space-y-5 rounded-2xl border border-white/10 bg-ink-900/80 p-6 shadow-2xl" onsubmit={login}>
    <label class="block">
      <span class="mb-2 block text-sm text-slate-300">Username</span>
      <input class="w-full rounded-lg border border-white/10 bg-ink-950 px-3 py-2.5 outline-none focus:border-mint-400" bind:value={username} autocomplete="username" required />
    </label>
    <label class="block">
      <span class="mb-2 block text-sm text-slate-300">Password</span>
      <input class="w-full rounded-lg border border-white/10 bg-ink-950 px-3 py-2.5 outline-none focus:border-mint-400" type="password" bind:value={password} autocomplete="current-password" required />
    </label>
    {#if error}<p class="text-sm text-red-300">{error}</p>{/if}
    <button class="w-full rounded-lg bg-mint-400 px-4 py-2.5 font-semibold text-ink-950 transition hover:bg-mint-500 disabled:opacity-50" disabled={submitting}>
      {submitting ? "Signing in…" : "Sign in"}
    </button>
  </form>
</main>

