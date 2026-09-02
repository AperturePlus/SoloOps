<script lang="ts">
  import type { RuntimeCheckpoint } from "$lib/contracts";

  let { checkpoint = null }: { checkpoint: RuntimeCheckpoint | null } = $props();

  const CHECKPOINT_LABELS: Record<RuntimeCheckpoint, string> = {
    preparing: "Preparing context",
    calling_model: "Thinking",
    executing_tools: "Executing tools",
    validating_completion: "Verifying results",
    reporting: "Writing report",
    waiting_for_approval: "Waiting for approval",
    retry_scheduled: "Retry scheduled",
    recovering: "Recovering"
  };

  const label = $derived(
    checkpoint ? (CHECKPOINT_LABELS[checkpoint] ?? checkpoint.replace(/_/g, " ")) : ""
  );
</script>

<div
  class="flex items-center gap-3 rounded-lg border border-edge bg-ink-800/60 px-3 py-2.5"
  role="status"
  aria-live="polite"
>
  <span class="flex items-center gap-1" aria-hidden="true">
    <span class="agent-dot size-1.5 rounded-full bg-mint-400"></span>
    <span class="agent-dot size-1.5 rounded-full bg-mint-400" style="animation-delay:0.18s"></span>
    <span class="agent-dot size-1.5 rounded-full bg-mint-400" style="animation-delay:0.36s"></span>
  </span>
  <span class="agent-shimmer-text text-sm font-medium">Working</span>
  {#if label}
    <span class="text-xs text-muted-foreground/65 transition-all duration-500">{label}</span>
  {/if}
</div>
