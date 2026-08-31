<script lang="ts">
  import { page } from "$app/state";
  import Icon from "$lib/components/Icon.svelte";
  let { children } = $props();

  const path = $derived(page.url.pathname);

  const NAV = [
    {
      href: "/settings/notifications",
      label: "Public IP notifications",
      icon: "mail" as const,
      note: "Email when your public IPv4 changes"
    },
    {
      href: "/settings/agents",
      label: "Agent runtime",
      icon: "layers" as const,
      note: "Model, pricing and approvals",
      placeholder: true
    },
    {
      href: "/settings/security",
      label: "Security",
      icon: "shield" as const,
      note: "Keys, sessions and policy",
      placeholder: true
    }
  ];
</script>

<div class="flex h-screen overflow-hidden">
  <!-- Left: settings rail -->
  <aside
    class="agent-scroll flex w-56 shrink-0 flex-col overflow-y-auto border-r border-edge bg-ink-950/50 p-2"
  >
    <div class="flex items-center gap-2 px-2 pb-2 pt-2">
      <Icon name="gear" size={14} class="text-slate-500" />
      <span class="text-[13px] font-semibold tracking-tight text-slate-100">Settings</span>
    </div>

    <nav class="mt-1 space-y-0.5" aria-label="Settings sections">
      {#each NAV as item (item.href)}
        {#if item.placeholder}
          <button
            type="button"
            class="rail-link cursor-default opacity-60"
            title="Not available in this build"
          >
            <Icon name={item.icon} size={14} />
            <span class="min-w-0 flex-1">
              <span class="block truncate">{item.label}</span>
              <span class="block truncate text-[10px] font-normal text-slate-600">
                {item.note}
              </span>
            </span>
            <Icon name="lock" size={11} class="shrink-0 text-slate-700" />
          </button>
        {:else}
          <a
            href={item.href}
            class="rail-link"
            data-active={path.startsWith(item.href)}
            aria-current={path.startsWith(item.href) ? "page" : undefined}
          >
            <Icon name={item.icon} size={14} />
            <span class="min-w-0 flex-1">
              <span class="block truncate">{item.label}</span>
              <span class="block truncate text-[10px] font-normal text-slate-600">
                {item.note}
              </span>
            </span>
            {#if path.startsWith(item.href)}
              <span class="size-1.5 shrink-0 rounded-full bg-mint-400"></span>
            {/if}
          </a>
        {/if}
      {/each}
    </nav>

    <div class="mt-auto px-2 pt-4">
      <p
        class="rounded-lg border border-mint-400/15 bg-mint-400/[0.05] px-2.5 py-2 text-[10.5px] leading-relaxed text-slate-500"
      >
        Owner-scoped settings apply to every run and are enforced server-side.
      </p>
    </div>
  </aside>

  <!-- Right: active section -->
  <div class="agent-scroll min-w-0 flex-1 overflow-y-auto">
    {@render children()}
  </div>
</div>
