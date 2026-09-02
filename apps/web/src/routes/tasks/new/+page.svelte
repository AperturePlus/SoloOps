<script lang="ts">
  import { goto } from "$app/navigation";
  import type { TaskSummary } from "$lib/contracts";
  import { api, ApiClientError } from "$lib/api";
  import Icon from "$lib/components/Icon.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Textarea } from "$lib/components/ui/textarea";

  let title = $state("");
  let goal = $state("");
  let error = $state("");
  let submitting = $state(false);

  async function createTask(event: SubmitEvent) {
    event.preventDefault();
    error = "";
    submitting = true;
    try {
      const task = await api<TaskSummary>("/api/tasks", {
        method: "POST",
        body: JSON.stringify({ title, goal })
      });
      await goto(`/runs/${task.latestRunId}`);
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not create task";
    } finally {
      submitting = false;
    }
  }

  const charWarning = $derived(goal.length > 0 && goal.length < 12);
</script>

<svelte:head>
  <title>New task · SoloOps</title>
</svelte:head>

<div class="ops-grid-bg p-6">
  <PageHeader
    crumbs={[{ label: "Tasks", href: "/tasks" }, { label: "New task" }]}
    title="Create a task"
    subtitle="Start a durable Agent run with approval-gated tools, verification, and an auditable report."
  >
    {#snippet actions()}
      <span
        class="hidden rounded-md border border-edge bg-ink-900/70 px-2.5 py-1 font-mono text-2xs text-muted-foreground/65 md:block"
      >
        lease: owner-local
      </span>
      <span
        class="hidden rounded-md border border-edge bg-ink-900/70 px-2.5 py-1 font-mono text-2xs text-muted-foreground/65 md:block"
      >
        mode: agent
      </span>
    {/snippet}
  </PageHeader>

  <div class="mt-6 grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_280px]">
    <form class="space-y-4" onsubmit={createTask}>
      <label class="block">
        <span class="mb-1.5 block text-sm font-medium text-foreground">Title</span>
        <Input
          bind:value={title}
          maxlength={160}
          placeholder="e.g. Redeploy the compose stack"
          required
        />
      </label>

      <label class="block">
        <span class="mb-1.5 block text-sm font-medium text-foreground">Goal</span>
        <Textarea
          class="min-h-52 resize-y leading-relaxed"
          bind:value={goal}
          maxlength={20000}
          placeholder="Describe the outcome you want, including any constraints…"
          required
        ></Textarea>
        <span
          class="mt-1.5 block text-right font-mono text-2xs tabular-nums text-muted-foreground/65"
          >{goal.length}/20000</span
        >
      </label>

      {#if charWarning}
        <p class="text-xs text-amber-300/90">
          A short goal may be underspecified — include verification criteria for best results.
        </p>
      {/if}
      {#if error}<p class="text-sm text-red-300">{error}</p>{/if}

      <Button type="submit" disabled={submitting}>
        {#if submitting}
          <span
            class="size-3.5 animate-spin rounded-full border-2 border-ink-950/20 border-t-ink-950"
          ></span>
          Creating…
        {:else}
          <Icon name="play" size={14} />
          Create task
        {/if}
      </Button>
    </form>

    <aside class="space-y-4">
      <div class="panel p-4">
        <p class="text-2xs font-semibold uppercase tracking-[0.12em] text-muted-foreground/65">
          What happens next
        </p>
        <ol class="mt-3 space-y-3">
          {#each [{ icon: "spark", text: "Agent plans the work and updates the plan panel." }, { icon: "terminal", text: "Tools run approval-gated, one at a time." }, { icon: "file", text: "Evidence and a final report are produced." }] as step (step.text)}
            <li class="flex gap-2.5">
              <span
                class="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md border border-edge bg-ink-800/60 text-muted-foreground"
              >
                <Icon name={step.icon as "spark" | "terminal" | "file"} size={12} />
              </span>
              <span class="text-xs leading-relaxed text-muted-foreground">{step.text}</span>
            </li>
          {/each}
        </ol>
      </div>
      <div class="panel p-4">
        <p
          class="flex items-center gap-1.5 text-2xs font-semibold uppercase tracking-[0.12em] text-muted-foreground/65"
        >
          <Icon name="shield" size={12} />
          Safety
        </p>
        <p class="mt-2 text-xs leading-relaxed text-muted-foreground">
          Every workspace write, process and network action passes policy. Nothing runs with root.
        </p>
      </div>
      <div class="panel p-4">
        <p class="text-2xs font-semibold uppercase tracking-[0.12em] text-muted-foreground/65">
          Default budget
        </p>
        <div class="command-strip mt-3">
          <div class="px-3 py-2">
            <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
              Turns
            </p>
            <p class="mt-0.5 font-mono text-sm text-muted-foreground">8</p>
          </div>
          <div class="px-3 py-2">
            <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
              Tools
            </p>
            <p class="mt-0.5 font-mono text-sm text-muted-foreground">6</p>
          </div>
          <div class="px-3 py-2">
            <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
              Duration
            </p>
            <p class="mt-0.5 font-mono text-sm text-muted-foreground">60s</p>
          </div>
        </div>
        <p class="mt-2.5 text-2xs leading-relaxed text-muted-foreground/65">
          Budgets are enforced server-side on every run.
        </p>
      </div>
    </aside>
  </div>
</div>
