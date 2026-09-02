<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { api, ApiClientError } from "$lib/api";
  import type { IpNotificationSettings, TestIpNotificationResponse } from "$lib/contracts";
  import { formatDateTime, relativeTime } from "$lib/time";
  import Icon from "$lib/components/Icon.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Textarea } from "$lib/components/ui/textarea";
  import { Switch } from "$lib/components/ui/switch";

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

  function dateTitle(value: number | null): string {
    return value === null ? "Never" : formatDateTime(value);
  }

  function date(value: number | null): string {
    return value === null ? "Never" : relativeTime(value);
  }
</script>

<svelte:head>
  <title>Public IP notifications · SoloOps</title>
</svelte:head>

<div class="p-6">
  <PageHeader
    crumbs={[{ label: "Settings" }, { label: "Notifications" }]}
    title="Public IP notifications"
    subtitle="Email the current public IPv4 address on first detection and whenever it changes."
  />

  {#if loading}
    <div class="mt-6 grid gap-4 lg:grid-cols-2">
      <div class="h-44 animate-pulse rounded-lg bg-ink-800/70"></div>
      <div class="h-44 animate-pulse rounded-lg bg-ink-800/70"></div>
    </div>
  {:else if settings}
    <div class="mt-6 grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_320px]">
      <!-- Left: configuration form -->
      <form class="space-y-4 min-w-0" onsubmit={save}>
        <div class="panel p-5">
          <div class="flex items-center justify-between gap-4">
            <span>
              <span class="block text-sm font-medium text-foreground">Enable notifications</span>
              <span class="mt-1 block text-sm text-muted-foreground"
                >Checks every five minutes by default.</span
              >
            </span>
            <Switch
              bind:checked={enabled}
              aria-label="Enable notifications"
              disabled={!settings.smtpConfigured}
            />
          </div>
          {#if !settings.smtpConfigured}
            <p
              class="mt-4 rounded-lg border border-amber-400/30 bg-amber-400/10 p-3 text-sm text-amber-300"
            >
              SMTP is not configured on the server. Recipients can be saved while notifications are
              disabled.
            </p>
          {/if}
        </div>

        <label class="block">
          <span class="mb-1.5 block text-sm font-medium text-foreground">
            Recipient email addresses
          </span>
          <Textarea
            class="min-h-32 resize-y leading-relaxed"
            bind:value={recipientsText}
            placeholder="owner@example.com&#10;backup@example.com"
          ></Textarea>
          <span class="mt-2 block text-xs text-muted-foreground/65"
            >One per line, up to 20 addresses.</span
          >
        </label>

        {#if error}<p class="text-sm text-red-300">{error}</p>{/if}
        {#if message}<p class="text-sm text-mint-400">{message}</p>{/if}

        <div class="flex flex-wrap gap-3">
          <Button type="submit" disabled={saving}>
            {saving ? "Saving…" : "Save settings"}
          </Button>
          <Button
            variant="outline"
            type="button"
            onclick={sendTest}
            disabled={testing || !settings.smtpConfigured || settings.recipients.length === 0}
          >
            {testing ? "Sending…" : "Send test email"}
          </Button>
        </div>
      </form>

      <!-- Right: detection status rail -->
      <div class="space-y-4">
        <section class="panel p-5">
          <div class="flex items-center gap-2">
            <Icon name="server" size={14} class="text-muted-foreground/65" />
            <h2 class="text-sm font-semibold tracking-tight text-foreground">Detection status</h2>
          </div>
          <div class="command-strip mt-3">
            <div class="px-3 py-2.5">
              <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
                Current IPv4
              </p>
              <p class="mt-0.5 truncate font-mono text-sm text-foreground">
                {settings.currentIpv4 ?? "Unknown"}
              </p>
            </div>
            <div class="px-3 py-2.5">
              <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
                Last checked
              </p>
              <p
                class="mt-0.5 text-sm text-muted-foreground"
                title={dateTitle(settings.lastCheckedAt)}
              >
                {date(settings.lastCheckedAt)}
              </p>
            </div>
            <div class="px-3 py-2.5">
              <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
                Last changed
              </p>
              <p
                class="mt-0.5 text-sm text-muted-foreground"
                title={dateTitle(settings.lastChangedAt)}
              >
                {date(settings.lastChangedAt)}
              </p>
            </div>
          </div>
          <div class="mt-3 flex items-center gap-1.5 text-2xs text-muted-foreground/65">
            <span
              class={`size-1.5 rounded-full ${settings.enabled ? "agent-live-ring bg-mint-400" : "bg-muted-foreground/65"}`}
            ></span>
            {settings.enabled ? "Notifications active" : "Notifications disabled"}
          </div>
        </section>

        <section class="panel p-5">
          <div class="flex items-center justify-between">
            <h2 class="text-sm font-semibold tracking-tight text-foreground">Recipients</h2>
            <span class="font-mono text-2xs text-muted-foreground/65"
              >{settings.recipients.length}</span
            >
          </div>
          {#if settings.recipients.length > 0}
            <ul class="agent-scroll mt-3 max-h-64 space-y-2 overflow-y-auto pr-1">
              {#each settings.recipients as recipient (recipient.email)}
                <li
                  class="flex items-start justify-between gap-2 rounded-lg border border-edge px-3 py-2 text-sm"
                >
                  <span class="flex min-w-0 items-center gap-2 text-foreground">
                    <Icon name="mail" size={12} class="shrink-0 text-muted-foreground/65" />
                    <span class="truncate">{recipient.email}</span>
                  </span>
                  <span
                    class="shrink-0 text-right text-2xs text-muted-foreground/65"
                    title={dateTitle(recipient.lastNotifiedAt)}
                  >
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
            <p class="mt-3 text-sm leading-relaxed text-muted-foreground/65">
              No recipients yet. Add addresses on the left and save.
            </p>
          {/if}
        </section>
      </div>
    </div>
  {:else if error}
    <p class="mt-6 text-sm text-red-300">{error}</p>
  {/if}
</div>
