<script lang="ts">
  import type { RunStatus } from "$lib/contracts";

  // The single entry point for status display (ui-spec.md §3.1).
  // - default: pill with live dot, for headers / detail views.
  // - compact: small pill without dot, for dense list rows.
  let { status, compact = false }: { status: RunStatus; compact?: boolean } = $props();

  const TERMINAL: ReadonlySet<RunStatus> = new Set(["succeeded", "failed", "cancelled"]);

  const STYLES: Record<RunStatus, string> = {
    draft: "border-edge bg-muted text-muted-foreground",
    queued: "border-sky-400/30 bg-sky-400/10 text-sky-300",
    leased: "border-sky-400/30 bg-sky-400/10 text-sky-300",
    planning: "border-sky-400/30 bg-sky-400/10 text-sky-300",
    running: "border-mint-400/30 bg-mint-400/10 text-mint-400",
    waiting_for_approval: "border-amber-400/30 bg-amber-400/10 text-amber-300",
    paused: "border-sky-400/30 bg-sky-400/10 text-sky-300",
    verifying: "border-sky-400/30 bg-sky-400/10 text-sky-300",
    reporting: "border-sky-400/30 bg-sky-400/10 text-sky-300",
    retry_scheduled: "border-amber-400/30 bg-amber-400/10 text-amber-300",
    blocked: "border-amber-400/30 bg-amber-400/10 text-amber-300",
    needs_recovery: "border-red-400/30 bg-red-400/10 text-red-300",
    succeeded: "border-mint-400/30 bg-mint-400/10 text-mint-400",
    failed: "border-red-400/30 bg-red-400/10 text-red-300",
    cancelled: "border-edge bg-muted text-muted-foreground"
  };

  const LABELS: Record<RunStatus, string> = {
    draft: "Draft",
    queued: "Queued",
    leased: "Leased",
    planning: "Planning",
    running: "Running",
    waiting_for_approval: "Awaiting approval",
    paused: "Paused",
    verifying: "Verifying",
    reporting: "Reporting",
    retry_scheduled: "Retry scheduled",
    blocked: "Blocked",
    needs_recovery: "Needs recovery",
    succeeded: "Succeeded",
    failed: "Failed",
    cancelled: "Cancelled"
  };

  // Compact rows use curated short words — never character truncation.
  const SHORT_LABELS: Partial<Record<RunStatus, string>> = {
    waiting_for_approval: "Approval",
    needs_recovery: "Recovery",
    retry_scheduled: "Retry"
  };

  const live = $derived(!TERMINAL.has(status));
  const label = $derived(compact ? (SHORT_LABELS[status] ?? LABELS[status]) : LABELS[status]);
</script>

{#if compact}
  <span
    class={`inline-flex shrink-0 items-center rounded-full border px-2 py-0.5 text-2xs font-medium transition-colors duration-300 ${STYLES[status]}`}
    title={LABELS[status]}
  >
    {label}
  </span>
{:else}
  <span
    class={`inline-flex items-center gap-2 rounded-full border px-3 py-1 text-xs font-medium transition-colors duration-300 ${STYLES[status]}`}
  >
    <span class={`size-1.5 rounded-full bg-current ${live ? "animate-pulse" : "opacity-80"}`}
    ></span>
    {label}
  </span>
{/if}
