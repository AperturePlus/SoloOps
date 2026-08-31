<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { api, ApiClientError } from "$lib/api";
  import type { IpNotificationSettings, TestIpNotificationResponse } from "$lib/contracts";
  import Icon from "$lib/components/Icon.svelte";

  let settings = $state<IpNotificationSettings | null>(null);
  let enabled = $state(false);
  let recipientsText = $state("");
  let loading = $state(true);
  let saving = $state(false);
  let testing = $state(false);
  let error = $state("");
  let message = $state("");

  onMount(load);

  async function load() {
    try {
      settings = await api<IpNotificationSettings>("/api/settings/ip-notifications");
      enabled = settings.enabled;
      recipientsText = settings.recipients.map((recipient) => recipient.email).join("\n");
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not load notification settings";
    } finally {
      loading = false;
    }
  }

  function recipientList(): string[] {
    return recipientsText
      .split(/[\n,;]/)
      .map((recipient) => recipient.trim())
      .filter(Boolean);
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    saving = true;
    error = "";
    message = "";
    try {
      settings = await api<IpNotificationSettings>("/api/settings/ip-notifications", {
        method: "PUT",
        body: JSON.stringify({ enabled, recipients: recipientList() })
      });
      enabled = settings.enabled;
      recipientsText = settings.recipients.map((recipient) => recipient.email).join("\n");
      message = "Notification settings saved.";
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not save notification settings";
    } finally {
      saving = false;
    }
  }

  async function sendTest() {
    testing = true;
    error = "";
    message = "";
    try {
      const result = await api<TestIpNotificationResponse>("/api/settings/ip-notifications/test", {
        method: "POST"
      });
      message = `Test email sent to ${result.sentCount} recipient${result.sentCount === 1 ? "" : "s"}.`;
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not send test email";
    } finally {
      testing = false;
    }
  }

  function date(value: number | null): string {
    return value === null ? "Never" : new Date(value).toLocaleString();
  }
</script>

<svelte:head>
  <title>Public IP notifications · SoloOps</title>
</svelte:head>

<div class="p-6">
  <div class="flex items-start gap-3">
    <span
      class="mt-0.5 grid size-9 shrink-0 place-items-center rounded-lg border border-edge bg-ink-900/70 text-slate-300"
    >
      <Icon name="mail" size={16} />
    </span>
    <div>
      <p class="text-[11px] font-medium uppercase tracking-[0.2em] text-mint-400">Owner settings</p>
      <h1 class="mt-1 text-xl font-semibold tracking-tight text-slate-100">
        Public IP notifications
      </h1>
      <p class="mt-1 text-[12.5px] leading-relaxed text-slate-400">
        Email the current public IPv4 address on first detection and whenever it changes.
      </p>
    </div>
  </div>

  {#if loading}
    <div class="mt-6 grid gap-4 lg:grid-cols-2">
      <div class="h-44 animate-pulse rounded-xl bg-white/[0.04]"></div>
      <div class="h-44 animate-pulse rounded-xl bg-white/[0.04]"></div>
    </div>
  {:else if settings}
    <div class="mt-6 grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_320px]">
      <!-- Left: configuration form -->
      <form class="space-y-4 min-w-0" onsubmit={save}>
        <div class="panel p-5">
          <label class="flex cursor-pointer items-center justify-between gap-5">
            <span>
              <span class="block text-[14px] font-medium text-slate-100">Enable notifications</span>
              <span class="mt-1 block text-[13px] text-slate-400"
                >Checks every five minutes by default.</span
              >
            </span>
            <input
              class="size-4.5 accent-mint-400 cursor-pointer"
              type="checkbox"
              bind:checked={enabled}
            />
          </label>
          {#if !settings.smtpConfigured}
            <p
              class="mt-4 rounded-lg border border-amber-400/30 bg-amber-400/10 p-3 text-[13px] text-amber-300"
            >
              SMTP is not configured on the server. Recipients can be saved while notifications are
              disabled.
            </p>
          {/if}
        </div>

        <label class="block">
          <span class="mb-1.5 block text-[13px] font-medium text-slate-300">
            Recipient email addresses
          </span>
          <textarea
            class="min-h-32 w-full resize-y rounded-lg border border-edge bg-ink-900/80 px-3 py-2.5 text-[13px] leading-relaxed text-slate-100 placeholder:text-slate-600 outline-none transition-colors focus:border-mint-400/50"
            bind:value={recipientsText}
            placeholder="owner@example.com&#10;backup@example.com"></textarea>
          <span class="mt-2 block text-xs text-slate-500">One per line, up to 20 addresses.</span>
        </label>

        {#if error}<p class="text-[13px] text-red-300">{error}</p>{/if}
        {#if message}<p class="text-[13px] text-mint-400">{message}</p>{/if}

        <div class="flex flex-wrap gap-3">
          <button
            class="flex items-center gap-1.5 rounded-md bg-mint-400 px-4 py-2 text-[13px] font-semibold text-ink-950 transition-all hover:bg-mint-500 active:scale-[0.98] disabled:opacity-50"
            disabled={saving}
          >
            {saving ? "Saving…" : "Save settings"}
          </button>
          <button
            class="rounded-md border border-white/10 px-4 py-2 text-[13px] text-slate-200 transition-colors hover:bg-white/5 disabled:opacity-50"
            type="button"
            onclick={sendTest}
            disabled={testing || !settings.smtpConfigured || settings.recipients.length === 0}
          >
            {testing ? "Sending…" : "Send test email"}
          </button>
        </div>
      </form>

      <!-- Right: detection status rail -->
      <div class="space-y-4">
        <section class="panel p-5">
          <div class="flex items-center gap-2">
            <Icon name="server" size={14} class="text-slate-500" />
            <h2 class="text-[13px] font-semibold tracking-tight text-slate-200">
              Detection status
            </h2>
          </div>
          <div class="command-strip mt-3">
            <div class="px-3 py-2.5">
              <p class="text-[9.5px] font-medium uppercase tracking-[0.12em] text-slate-600">
                Current IPv4
              </p>
              <p class="mt-0.5 truncate font-mono text-[13px] text-slate-200">
                {settings.currentIpv4 ?? "Unknown"}
              </p>
            </div>
            <div class="px-3 py-2.5">
              <p class="text-[9.5px] font-medium uppercase tracking-[0.12em] text-slate-600">
                Last checked
              </p>
              <p class="mt-0.5 text-[13px] text-slate-300">{date(settings.lastCheckedAt)}</p>
            </div>
            <div class="px-3 py-2.5">
              <p class="text-[9.5px] font-medium uppercase tracking-[0.12em] text-slate-600">
                Last changed
              </p>
              <p class="mt-0.5 text-[13px] text-slate-300">{date(settings.lastChangedAt)}</p>
            </div>
          </div>
          <div class="mt-3 flex items-center gap-1.5 text-[11px] text-slate-500">
            <span
              class={`size-1.5 rounded-full ${settings.enabled ? "agent-live-ring bg-mint-400" : "bg-slate-600"}`}
            ></span>
            {settings.enabled ? "Notifications active" : "Notifications disabled"}
          </div>
        </section>

        <section class="panel p-5">
          <div class="flex items-center justify-between">
            <h2 class="text-[13px] font-semibold tracking-tight text-slate-200">Recipients</h2>
            <span class="font-mono text-[10px] text-slate-600">{settings.recipients.length}</span>
          </div>
          {#if settings.recipients.length > 0}
            <ul class="agent-scroll mt-3 max-h-64 space-y-2 overflow-y-auto pr-1">
              {#each settings.recipients as recipient (recipient.email)}
                <li
                  class="flex items-start justify-between gap-2 rounded-lg border border-white/5 px-3 py-2 text-[12.5px]"
                >
                  <span class="flex min-w-0 items-center gap-2 text-slate-200">
                    <Icon name="mail" size={12} class="shrink-0 text-slate-500" />
                    <span class="truncate">{recipient.email}</span>
                  </span>
                  <span class="shrink-0 text-right text-[11px] text-slate-500">
                    {#if recipient.lastError}
                      <span class="text-red-300">Last delivery failed</span>
                    {:else}
                      Last sent: {date(recipient.lastNotifiedAt)}
                    {/if}
                  </span>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="mt-3 text-[12.5px] leading-relaxed text-slate-500">
              No recipients yet. Add addresses on the left and save.
            </p>
          {/if}
        </section>
      </div>
    </div>
  {:else if error}
    <p class="mt-6 text-[13px] text-red-300">{error}</p>
  {/if}
</div>
