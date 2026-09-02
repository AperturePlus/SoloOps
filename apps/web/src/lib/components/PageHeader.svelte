<script lang="ts">
  // Unified page header (ui-spec.md §2): breadcrumb › title › subtitle + actions.
  // Every route must open with this component. Do not hand-roll page headers.
  import Icon from "./Icon.svelte";

  export interface Crumb {
    label: string;
    href?: string;
  }

  let {
    crumbs = [],
    title,
    subtitle = "",
    subtitleClass = "",
    back = "",
    actions
  }: {
    crumbs?: Crumb[];
    title: string;
    subtitle?: string;
    subtitleClass?: string;
    back?: string;
    actions?: import("svelte").Snippet;
  } = $props();
</script>

<header class="flex items-start justify-between gap-4">
  <div class="min-w-0 flex-1">
    {#if crumbs.length > 0}
      <nav class="flex items-center gap-1.5 text-sm" aria-label="Breadcrumb">
        {#each crumbs as crumb, i (i)}
          {#if i > 0}
            <span class="text-muted-foreground/65" aria-hidden="true">›</span>
          {/if}
          {#if crumb.href}
            <a
              href={crumb.href}
              class="text-muted-foreground/65 transition-colors hover:text-foreground"
            >
              {crumb.label}
            </a>
          {:else}
            <span class="text-foreground">{crumb.label}</span>
          {/if}
        {/each}
      </nav>
    {/if}

    <div class="mt-1.5 flex items-center gap-2.5">
      {#if back}
        <a
          href={back}
          class="grid size-8 shrink-0 place-items-center rounded-lg border border-edge bg-ink-900/70 text-muted-foreground transition-colors hover:border-edge-strong hover:text-foreground"
          aria-label="Back"
        >
          <Icon name="arrow-left" size={16} />
        </a>
      {/if}
      <h1 class="truncate text-xl font-semibold tracking-tight text-foreground">{title}</h1>
    </div>

    {#if subtitle}
      <p class={`mt-1 text-sm text-muted-foreground ${subtitleClass}`}>{subtitle}</p>
    {/if}
  </div>

  {#if actions}
    <div class="flex shrink-0 items-center gap-2">
      {@render actions()}
    </div>
  {/if}
</header>
