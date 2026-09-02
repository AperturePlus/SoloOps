<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import type { EventEnvelope } from "$lib/contracts";
  import { relativeTime } from "$lib/time";
  import Icon, { type IconName } from "./Icon.svelte";

  let {
    events,
    toolNames = {},
    pageSize = 60
  }: {
    events: EventEnvelope[];
    toolNames?: Record<string, string>;
    pageSize?: number;
  } = $props();

  let now = $state(Date.now());
  let expanded = new SvelteSet<string>();

  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), 15_000);
    return () => clearInterval(timer);
  });

  // Windowed view: only render the most recent `pageSize` events; growing the
  // store no longer grows the DOM. A footer button reveals older batches.
  const visible = $derived(events.slice(Math.max(0, events.length - pageSize)));
  const hiddenCount = $derived(events.length - pageSize);

  function revealMore() {
    pageSize += 60;
  }

  function toggleRaw(id: string) {
    if (expanded.has(id)) expanded.delete(id);
    else expanded.add(id);
  }

  function eventTone(type: EventEnvelope["type"]): string {
    switch (type) {
      case "run.created":
        return "text-muted-foreground/65";
      case "run.status_changed":
        return "text-sky-300";
      case "agent.plan_updated":
        return "text-sky-300";
      case "agent.message":
        return "text-mint-400";
      case "tool.call_started":
        return "text-sky-300";
      case "tool.call_completed":
        return "text-mint-400";
      case "tool.call_failed":
        return "text-red-300";
      case "run.reported":
        return "text-mint-400";
      default:
        return "text-muted-foreground/65";
    }
  }

  function eventIcon(type: EventEnvelope["type"]): IconName {
    if (type === "run.status_changed") return "activity";
    if (type.startsWith("tool.")) return "terminal";
    if (type === "agent.message") return "spark";
    if (type === "agent.plan_updated") return "list";
    if (type === "run.created") return "circle";
    if (type === "run.reported") return "check";
    return "circle";
  }

  function summarize(event: EventEnvelope): { title: string; detail: string } {
    const payload = event.payload ?? {};
    const str = (key: string): string =>
      typeof payload[key] === "string" ? (payload[key] as string) : "";

    switch (event.type) {
      case "run.created":
        return { title: "Run created", detail: "" };
      case "run.status_changed": {
        const to = str("to").replace(/_/g, " ") || "unknown";
        return { title: `Status → ${to}`, detail: str("reason") };
      }
      case "agent.plan_updated":
        return { title: "Plan updated", detail: "" };
      case "agent.message":
        return { title: "Agent", detail: str("preview") };
      case "tool.call_started":
      case "tool.call_completed":
      case "tool.call_failed": {
        const callId = str("callId");
        const name = toolNames[callId] ?? callId;
        const verb =
          event.type === "tool.call_started"
            ? "Tool started"
            : event.type === "tool.call_completed"
              ? "Tool completed"
              : "Tool failed";
        const detail =
          str("summary") ||
          str("resultSummary") ||
          [str("category"), str("preview")].filter(Boolean).join(" · ");
        return { title: `${verb}${name ? `: ${name}` : ""}`, detail };
      }
      case "run.reported":
        return { title: "Final report ready", detail: str("summary") };
      default:
        return { title: event.type, detail: "" };
    }
  }
</script>

<section aria-label="Activity">
  {#if hiddenCount > 0}
    <button
      type="button"
      class="mt-2 flex w-full items-center justify-center gap-1.5 rounded-lg border border-dashed border-edge py-1.5 text-2xs text-muted-foreground/65 transition-colors hover:border-edge-strong hover:text-muted-foreground"
      onclick={revealMore}
    >
      <Icon name="chevron-right" size={12} />
      Show {hiddenCount} earlier events
    </button>
  {/if}

  <ol class="mt-2 space-y-px">
    {#each visible as event, index (event.sequence)}
      {@const { title, detail } = summarize(event)}
      <li
        class="animate-fade-up group rounded-md px-2 py-1.5 transition-colors duration-200 hover:bg-ink-800/60"
        style={`animation-delay:${Math.min(index * 30, 240)}ms`}
      >
        <div class="flex items-start gap-2.5">
          <span
            class={`mt-0.5 grid size-4 shrink-0 place-items-center ${eventTone(event.type)}`}
            aria-hidden="true"
          >
            <Icon name={eventIcon(event.type)} size={12} />
          </span>

          <div class="min-w-0 flex-1">
            <div class="flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
              <span class="text-sm text-foreground">{title}</span>
              <time
                class="text-2xs tabular-nums text-muted-foreground/65"
                datetime={new Date(event.createdAt).toISOString()}
                title={new Date(event.createdAt).toLocaleString()}
              >
                {relativeTime(event.createdAt, now)}
              </time>
              <span class="ml-auto font-mono text-2xs text-muted-foreground/65"
                >#{event.sequence}</span
              >
            </div>
            {#if detail}
              <p class="mt-0.5 text-xs leading-relaxed break-words text-muted-foreground">
                {detail}
              </p>
            {/if}

            <button
              type="button"
              class="mt-0.5 text-2xs text-muted-foreground/65 opacity-0 transition-opacity duration-200 hover:text-muted-foreground group-hover:opacity-100 focus:opacity-100"
              aria-expanded={expanded.has(event.id)}
              onclick={() => toggleRaw(event.id)}
            >
              {expanded.has(event.id) ? "Hide payload" : "Raw payload"}
            </button>
            <div class="agent-disclose" data-open={expanded.has(event.id)}>
              <div>
                <pre
                  class="agent-scroll mt-1 max-h-56 overflow-auto rounded-lg border border-edge bg-black/25 p-2 text-2xs leading-relaxed text-muted-foreground/65">{JSON.stringify(
                    event.payload,
                    null,
                    2
                  )}</pre>
              </div>
            </div>
          </div>
        </div>
      </li>
    {/each}
  </ol>
</section>
