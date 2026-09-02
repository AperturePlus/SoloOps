<script lang="ts">
  import "../app.css";
  import { page } from "$app/state";
  import Icon from "$lib/components/Icon.svelte";
  let { children } = $props();

  const path = $derived(page.url.pathname as string);
  const isHome = $derived(path === "/");
  const isTasks = $derived(path === "/tasks" || path.startsWith("/runs"));
  const isNew = $derived(path === "/tasks/new");
  const isSettings = $derived(path.startsWith("/settings"));

  // Shared nav item classes (ui-spec.md §2): 16px icons, one active treatment.
  const NAV_ITEM =
    "grid size-9 place-items-center rounded-lg text-muted-foreground transition-colors duration-200 hover:bg-ink-800/70 hover:text-foreground";
  const navClass = (active: boolean) =>
    `${NAV_ITEM}${active ? " bg-ink-800/70 text-foreground" : ""}`;
</script>

<svelte:head>
  <title>SoloOps</title>
  <meta name="description" content="Private, controlled agent operations" />
</svelte:head>

<div class="flex min-h-screen">
  <!-- Left rail navigation (Codex-style) -->
  <aside
    class="fixed inset-y-0 left-0 z-40 flex w-14 flex-col items-center border-r border-edge bg-ink-950/80 py-3 backdrop-blur-xl"
  >
    <a
      href="/tasks"
      class="grid size-8 place-items-center rounded-lg text-mint-400 transition-colors duration-200 hover:bg-ink-800/70"
      aria-label="SoloOps home"
      title="SoloOps"
    >
      <svg
        viewBox="0 0 20 20"
        class="size-5"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <rect x="2.5" y="2.5" width="15" height="15" rx="4" />
        <path d="M7 7h6M7 10.5h4" />
        <circle cx="13.5" cy="13.5" r="1" fill="currentColor" />
      </svg>
    </a>

    <nav class="mt-6 flex flex-1 flex-col items-center gap-1.5" aria-label="Primary">
      <a href="/" class={navClass(isHome)} aria-label="Overview" title="Overview">
        <Icon name="layout" size={16} />
      </a>
      <a href="/tasks" class={navClass(isTasks)} aria-label="Tasks" title="Tasks">
        <Icon name="list" size={16} />
      </a>
      <a href="/tasks/new" class={navClass(isNew)} aria-label="New task" title="New task">
        <Icon name="plus" size={16} />
      </a>
    </nav>

    <nav class="flex flex-col items-center gap-1.5" aria-label="System">
      <a
        href="/settings/notifications"
        class={navClass(isSettings)}
        aria-label="Settings"
        title="Settings"
      >
        <Icon name="gear" size={16} />
      </a>
    </nav>

    <span
      class="mt-3 rounded-full border border-mint-400/20 bg-mint-400/5 px-1.5 py-0.5 text-2xs font-semibold tracking-wider text-mint-400"
      title="SoloOps Phase 3.1"
    >
      3.1
    </span>
  </aside>

  <!-- Scrollable content zone -->
  <div class="min-w-0 flex-1 pl-14">
    {@render children()}
  </div>
</div>
