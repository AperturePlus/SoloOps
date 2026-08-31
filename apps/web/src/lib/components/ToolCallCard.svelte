<script lang="ts">
  import type { ToolCallSummary } from "$lib/contracts";
  import { formatDuration } from "$lib/time";
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

  $effect(() => {
    if (!live) return;
    const timer = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(timer);
  });

  const duration = $derived(
    call.startedAt ? formatDuration((call.completedAt ?? now) - call.startedAt) : ""
  );

  const RISK_TONES: Record<ToolCallSummary["risk"], string> = {
    read_only: "border-white/10 bg-white/5 text-slate-400",
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
      : "border-white/10 bg-ink-900/60 hover:border-white/20"
  }`}
>
  <button
    type="button"
    class="flex w-full items-center gap-2.5 px-3 py-2.5 text-left transition-colors duration-200 hover:bg-white/[0.02]"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <span class="grid size-5 shrink-0 place-items-center" aria-hidden="true">
      {#if live}
        <span class="size-3.5 animate-spin rounded-full border-2 border-white/10 border-t-mint-400"
        ></span>
      {:else if awaiting}
        <span class="agent-live-ring size-2 rounded-full bg-amber-300"></span>
      {:else if call.status === "completed"}
        <span
          class="animate-pop-in grid size-4.5 place-items-center rounded-full bg-mint-400/15 text-mint-400"
        >
          <Icon name="check" size={10} />
        </span>
      {:else if call.status === "failed" || call.status === "denied"}
        <span
          class="animate-pop-in grid size-4.5 place-items-center rounded-full bg-red-400/15 text-red-300"
        >
          <Icon name="x" size={10} />
        </span>
      {:else}
        <span class="size-1.5 rounded-full bg-slate-600"></span>
      {/if}
    </span>

    <span class="min-w-0 flex-1">
      <span class="block truncate font-mono text-[13px] font-medium text-slate-100">
        {call.name}
      </span>
      <span class="mt-0.5 flex flex-wrap items-center gap-1.5 text-[11px] text-slate-500">
        <span
          class={`rounded-full border px-1.5 py-px text-[9px] leading-4 ${RISK_TONES[call.risk]}`}
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
      <span class="shrink-0 font-mono text-[11px] tabular-nums text-slate-500">{duration}</span>
    {/if}

    <Icon
      name="chevron-down"
      size={12}
      class={`shrink-0 text-slate-500 transition-transform duration-300 ${open ? "rotate-180" : ""}`}
    />
  </button>

  <div class="agent-disclose" data-open={open} id={`call-body-${call.callId}`}>
    <div>
      <div class="space-y-3 border-t border-white/5 px-3 py-2.5">
        {#if call.resultSummary}
          <p class="text-[13px] leading-relaxed text-slate-300">{call.resultSummary}</p>
        {/if}

        {#if awaiting && call.approvalPreview}
          <div class="rounded-lg border border-amber-400/20 bg-black/25 p-3">
            <p class="text-[11px] font-semibold uppercase tracking-wider text-amber-300">
              Approval required
            </p>
            <pre
              class="agent-scroll mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words text-xs leading-relaxed text-slate-300">{JSON.stringify(
                call.approvalPreview,
                null,
                2
              )}</pre>
          </div>
          <div class="flex gap-2">
            <button
              type="button"
              class="flex-1 rounded-md bg-mint-400 px-3 py-1.5 text-[13px] font-semibold text-ink-950 transition-all duration-200 hover:bg-mint-500 active:scale-[0.98]"
              onclick={() => onDecide?.(call, "approve")}
            >
              Approve
            </button>
            <button
              type="button"
              class="flex-1 rounded-md border border-red-400/30 px-3 py-1.5 text-[13px] text-red-200 transition-all duration-200 hover:bg-red-400/10 active:scale-[0.98]"
              onclick={() => onDecide?.(call, "deny")}
            >
              Deny
            </button>
          </div>
        {/if}

        <p class="break-all font-mono text-[10px] text-slate-600" title={call.argumentsSha256}>
          sha256 {call.argumentsSha256}
        </p>
      </div>
    </div>
  </div>
</article>