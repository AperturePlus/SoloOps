<script lang="ts">
  import type { ToolCallSummary } from "$lib/contracts";
  import { Button } from "$lib/components/ui/button";
  import Icon from "./Icon.svelte";

  let {
    calls,
    onDecide,
    busy = false
  }: {
    calls: ToolCallSummary[];
    onDecide: (call: ToolCallSummary, decision: "approve" | "deny") => void;
    busy?: boolean;
  } = $props();

  const RISK_TONES: Record<ToolCallSummary["risk"], string> = {
    read_only: "border-edge bg-ink-800/70 text-muted-foreground",
    workspace_write: "border-sky-400/25 bg-sky-400/10 text-sky-300",
    process: "border-violet-400/25 bg-violet-400/10 text-violet-300",
    network: "border-amber-400/25 bg-amber-400/10 text-amber-300",
    privileged: "border-red-400/30 bg-red-400/10 text-red-300"
  };
</script>

<!--
  Surfaced approval gate: the run's core owner decision must be the most
  prominent element on the page, never hidden inside a collapsed card.
-->
<section
  class="animate-fade-up overflow-hidden rounded-lg border border-amber-400/40 bg-amber-400/[0.07] shadow-[0_0_32px_-12px_rgb(252_211_77/0.35)]"
  role="alert"
  aria-label="Approval required"
>
  <div class="flex items-center gap-2.5 border-b border-amber-400/20 px-4 py-3">
    <span class="agent-live-ring size-2 shrink-0 rounded-full bg-amber-300"></span>
    <h2 class="text-sm font-semibold tracking-tight text-amber-100">
      Approval required{calls.length > 1 ? ` · ${calls.length} actions` : ""}
    </h2>
    <span class="ml-auto hidden items-center gap-1 text-2xs text-amber-200/60 sm:flex">
      <Icon name="clock" size={12} />
      Run is paused until you decide
    </span>
  </div>

  <div class="divide-y divide-amber-400/10">
    {#each calls as call (call.callId)}
      <div class="px-4 py-3.5">
        <div class="flex flex-wrap items-center gap-2">
          <span class="font-mono text-sm font-medium text-foreground">{call.name}</span>
          <span
            class={`rounded-full border px-1.5 py-px text-2xs leading-4 ${RISK_TONES[call.risk]}`}
          >
            {call.risk}
          </span>
          {#if call.policy}
            <span class="font-mono text-2xs text-muted-foreground/65">{call.policy}</span>
          {/if}
        </div>

        {#if call.approvalPreview}
          <pre
            class="agent-scroll mt-2.5 max-h-40 overflow-auto rounded-lg border border-amber-400/15 bg-black/30 p-2.5 text-2xs leading-relaxed text-muted-foreground">{JSON.stringify(
              call.approvalPreview,
              null,
              2
            )}</pre>
        {/if}

        <div class="mt-3 flex gap-2 sm:justify-end">
          <Button
            variant="destructive"
            size="lg"
            disabled={busy}
            onclick={() => onDecide(call, "deny")}
          >
            Deny
          </Button>
          <Button size="lg" disabled={busy} onclick={() => onDecide(call, "approve")}>
            <Icon name="check" size={12} />
            Approve
          </Button>
        </div>
      </div>
    {/each}
  </div>
</section>
