<script lang="ts">
  import type { ToolCallSummary } from "$lib/contracts";
  import { formatDuration } from "$lib/time";
  import { Button } from "$lib/components/ui/button";
  import Icon from "./Icon.svelte";

  let {
    call,
    onDecide
  }: {
    call: ToolCallSummary;
    onDecide?: (call: ToolCallSummary, decision: "approve" | "deny") => void;
  } = $props();

  let open = $state(false);
  let now = $state(Date.now());

  const live = $derived(call.status === "running" || call.status === "pending");
  const awaiting = $derived(call.status === "waiting_for_approval");

  // Approval actions must never hide behind the collapsed state: whenever a
  // call starts waiting for a decision, unfold it automatically.
  $effect(() => {
    if (awaiting) open = true;
  });

  $effect(() => {
    if (!live) return;
    const timer = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(timer);
  });

  const duration = $derived(
    call.startedAt ? formatDuration((call.completedAt ?? now) - call.startedAt) : ""
  );

  const RISK_TONES: Record<ToolCallSummary["risk"], string> = {
    read_only: "border-edge bg-ink-800/70 text-muted-foreground",
    workspace_write: "border-sky-400/25 bg-sky-400/10 text-sky-300",
    process: "border-violet-400/25 bg-violet-400/10 text-violet-300",
    network: "border-amber-400/25 bg-amber-400/10 text-amber-300",
    privileged: "border-red-400/30 bg-red-400/10 text-red-300"
  };
</script>

<article
  class={`animate-fade-up overflow-hidden rounded-lg border transition-all duration-300 ${
    awaiting
      ? "border-amber-400/30 bg-amber-400/[0.04]"
      : "border-edge bg-ink-900/60 hover:border-edge-strong"
  }`}
>
  <button
    type="button"
    class="flex w-full items-center gap-2.5 px-3 py-2.5 text-left transition-colors duration-200 hover:bg-ink-800/50"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <span class="grid size-5 shrink-0 place-items-center" aria-hidden="true">
      {#if live}
        <span class="size-3.5 animate-spin rounded-full border-2 border-edge border-t-mint-400"
        ></span>
      {:else if awaiting}
        <span class="agent-live-ring size-2 rounded-full bg-amber-300"></span>
      {:else if call.status === "completed"}
        <span
          class="animate-pop-in grid size-4.5 place-items-center rounded-full bg-mint-400/15 text-mint-400"
        >
          <Icon name="check" size={12} />
        </span>
      {:else if call.status === "failed" || call.status === "denied"}
        <span
          class="animate-pop-in grid size-4.5 place-items-center rounded-full bg-red-400/15 text-red-300"
        >
          <Icon name="x" size={12} />
        </span>
      {:else}
        <span class="size-1.5 rounded-full bg-muted-foreground/65"></span>
      {/if}
    </span>

    <span class="min-w-0 flex-1">
      <span class="block truncate font-mono text-sm font-medium text-foreground">
        {call.name}
      </span>
      <span class="mt-0.5 flex flex-wrap items-center gap-1.5 text-2xs text-muted-foreground/65">
        <span
          class={`rounded-full border px-1.5 py-px text-2xs leading-4 ${RISK_TONES[call.risk]}`}
        >
          {call.risk}
        </span>
        {#if call.errorCategory}
          <span class="text-red-300/80">{call.errorCategory}</span>
        {/if}
        {#if call.status === "waiting_for_approval"}
          <span class="text-amber-300/90">awaiting decision</span>
        {/if}
      </span>
    </span>

    {#if duration}
      <span class="shrink-0 font-mono text-2xs tabular-nums text-muted-foreground/65"
        >{duration}</span
      >
    {/if}

    <Icon
      name="chevron-down"
      size={12}
      class={`shrink-0 text-muted-foreground/65 transition-transform duration-300 ${open ? "rotate-180" : ""}`}
    />
  </button>

  <div class="agent-disclose" data-open={open} id={`call-body-${call.callId}`}>
    <div>
      <div class="space-y-3 border-t border-edge px-3 py-2.5">
        {#if call.resultSummary}
          <p class="text-sm leading-relaxed text-muted-foreground">{call.resultSummary}</p>
        {/if}

        {#if awaiting && call.approvalPreview}
          <div class="rounded-lg border border-amber-400/20 bg-black/25 p-3">
            <p class="text-2xs font-semibold uppercase tracking-wider text-amber-300">
              Approval required
            </p>
            <pre
              class="agent-scroll mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words text-xs leading-relaxed text-muted-foreground">{JSON.stringify(
                call.approvalPreview,
                null,
                2
              )}</pre>
          </div>
          <div class="flex gap-2">
            <Button class="flex-1" size="sm" onclick={() => onDecide?.(call, "approve")}>
              Approve
            </Button>
            <Button
              class="flex-1"
              variant="destructive"
              size="sm"
              onclick={() => onDecide?.(call, "deny")}
            >
              Deny
            </Button>
          </div>
        {/if}

        <p
          class="break-all font-mono text-2xs text-muted-foreground/65"
          title={call.argumentsSha256}
        >
          sha256 {call.argumentsSha256}
        </p>
      </div>
    </div>
  </div>
</article>
