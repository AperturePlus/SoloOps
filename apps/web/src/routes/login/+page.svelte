<script lang="ts">
  import { goto } from "$app/navigation";
  import { api } from "$lib/api";
  import Icon, { type IconName } from "$lib/components/Icon.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";

  let username = $state("owner");
  let password = $state("");
  let error = $state("");
  let submitting = $state(false);

  async function login(event: SubmitEvent) {
    event.preventDefault();
    error = "";
    submitting = true;
    try {
      await api("/api/auth/login", {
        method: "POST",
        body: JSON.stringify({ username, password })
      });
      await goto("/tasks");
    } catch (cause) {
      error = cause instanceof Error ? cause.message : "Login failed";
    } finally {
      submitting = false;
    }
  }

  const CAPABILITIES: { icon: IconName; title: string; text: string }[] = [
    {
      icon: "spark",
      title: "Plan first",
      text: "Every run opens with an explicit plan and budget before any tool executes."
    },
    {
      icon: "shield",
      title: "Approval-gated tools",
      text: "Workspace writes, processes and network calls pause for explicit owner review."
    },
    {
      icon: "eye",
      title: "Auditable evidence",
      text: "Every action leaves signed evidence and a final report you can replay."
    },
    {
      icon: "command",
      title: "Durable runs",
      text: "Runs survive restarts — outcomes, usage and reports are persisted end-to-end."
    }
  ];

  const FLOW = ["queued", "planning", "running", "reporting", "succeeded"];
</script>

<svelte:head>
  <title>Sign in · SoloOps</title>
</svelte:head>

<div class="flex min-h-screen">
  <!-- Left: brand / capability showcase (Codex-style split) -->
  <section
    class="ops-grid-bg relative hidden w-[44%] flex-col overflow-hidden border-r border-edge bg-ink-950/60 lg:flex"
  >
    <div
      class="pointer-events-none absolute -top-32 -left-32 size-96 rounded-full bg-mint-400/10 blur-3xl"
    ></div>

    <div class="relative z-10 flex flex-1 flex-col p-10">
      <div class="flex items-center gap-2.5">
        <span
          class="grid size-8 place-items-center rounded-lg border border-mint-400/25 bg-mint-400/10 text-mint-400"
        >
          <Icon name="command" size={16} />
        </span>
        <div>
          <p class="text-sm font-semibold tracking-tight text-foreground">SoloOps</p>
          <p class="text-2xs font-medium uppercase tracking-[0.18em] text-muted-foreground/65">
            Private control plane
          </p>
        </div>
      </div>

      <div class="mt-12 max-w-sm">
        <h1 class="text-2xl font-semibold leading-tight tracking-tight text-foreground">
          Run durable agents on your own machine.
        </h1>
        <p class="mt-3 text-sm leading-relaxed text-muted-foreground">
          A private execution queue for goal-driven work — planned, gated, measured and reported
          under your control.
        </p>
      </div>

      <ul class="mt-10 grid max-w-md gap-6 sm:grid-cols-2">
        {#each CAPABILITIES as item (item.icon)}
          <li class="flex gap-3">
            <span
              class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border border-edge bg-ink-800/60 text-muted-foreground"
            >
              <Icon name={item.icon} size={14} />
            </span>
            <div>
              <p class="text-sm font-semibold text-foreground">{item.title}</p>
              <p class="mt-1 text-xs leading-relaxed text-muted-foreground/65">{item.text}</p>
            </div>
          </li>
        {/each}
      </ul>
    </div>

    <!-- Run lifecycle flow strip -->
    <div class="relative z-10 border-t border-edge px-10 py-5">
      <div class="flex items-center gap-2.5">
        {#each FLOW as stage, index (stage)}
          <div class="flow-step" data-state={index < 3 ? "done" : index === 3 ? "current" : "idle"}>
            <span
              class={`size-1.5 rounded-full ${
                index < 3
                  ? "bg-mint-400"
                  : index === 3
                    ? "bg-foreground agent-live-ring"
                    : "bg-muted-foreground/65"
              }`}
            ></span>
            {stage}
            {#if index < FLOW.length - 1}
              <span class="h-px w-5 bg-edge-strong"></span>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  </section>

  <!-- Right: sign-in pane -->
  <main class="flex min-w-0 flex-1 items-center justify-center p-8">
    <div class="w-full max-w-sm">
      <p class="text-2xs font-medium uppercase tracking-[0.24em] text-mint-400">
        Private control plane
      </p>
      <h1 class="mt-2 text-2xl font-semibold tracking-tight text-foreground">Owner sign in</h1>
      <p class="mt-2 text-sm leading-relaxed text-muted-foreground">
        Use the Owner account initialized from the local terminal.
      </p>

      <form
        class="mt-8 space-y-4 rounded-lg border border-edge bg-ink-900/80 p-6 shadow-2xl"
        onsubmit={login}
      >
        <label class="block">
          <span class="mb-2 block text-sm text-foreground">Username</span>
          <Input bind:value={username} autocomplete="username" required />
        </label>
        <label class="block">
          <span class="mb-2 block text-sm text-foreground">Password</span>
          <Input bind:value={password} type="password" autocomplete="current-password" required />
        </label>
        {#if error}<p class="text-sm text-red-300">{error}</p>{/if}
        <Button class="w-full" type="submit" disabled={submitting}>
          {submitting ? "Signing in…" : "Sign in"}
        </Button>
      </form>

      <p class="mt-6 flex items-center justify-center gap-1.5 text-2xs text-muted-foreground/65">
        <Icon name="lock" size={12} />
        Session is local to this browser only.
      </p>
    </div>
  </main>
</div>
