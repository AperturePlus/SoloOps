<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { api, ApiClientError } from "$lib/api";
  import type { SmtpSettings, TestSmtpDeliveryResponse, UpdateSmtpSettingsRequest } from "$lib/contracts";
  import Icon from "$lib/components/Icon.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";

  let settings = $state<SmtpSettings | null>(null);
  let host = $state("");
  let port = $state(465);
  let security = $state<"tls" | "starttls">("tls");
  let from = $state("");
  let username = $state("");
  let password = $state("");
  let loading = $state(true);
  let saving = $state(false);
  let testing = $state(false);
  let clearing = $state(false);
  let testRecipient = $state("");
  let error = $state("");
  let message = $state("");

  onMount(load);

  async function load() {
    try {
      settings = await api<SmtpSettings>("/api/settings/smtp");
      if (settings.host) host = settings.host;
      if (settings.port) port = settings.port;
      if (settings.security === "starttls") security = "starttls";
      if (settings.from) from = settings.from;
      username = settings.username ?? "";
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not load SMTP settings";
    } finally {
      loading = false;
    }
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    saving = true;
    error = "";
    message = "";
    try {
      const body: UpdateSmtpSettingsRequest = {
        host: host.trim(),
        port: Number(port),
        security,
        from: from.trim(),
        username: username.trim() || null
      };
      // Only send the password when the owner typed a new one; an explicit
      // empty value clears the stored credential.
      if (password !== "") body.password = password;
      settings = await api<SmtpSettings>("/api/settings/smtp", {
        method: "PUT",
        body: JSON.stringify(body)
      });
      password = "";
      message = "SMTP settings saved.";
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not save SMTP settings";
    } finally {
      saving = false;
    }
  }

  async function clearSettings() {
    clearing = true;
    error = "";
    message = "";
    try {
      await api<void>("/api/settings/smtp", { method: "DELETE" });
      settings = await api<SmtpSettings>("/api/settings/smtp");
      password = "";
      message = "Saved settings removed; delivery falls back to .env configuration.";
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not clear SMTP settings";
    } finally {
      clearing = false;
    }
  }

  async function sendTest() {
    testing = true;
    error = "";
    message = "";
    try {
      const result = await api<TestSmtpDeliveryResponse>("/api/settings/smtp/test", {
        method: "POST",
        body: JSON.stringify({ recipient: testRecipient.trim() })
      });
      if (result.delivered) {
        message = `Test email delivered to ${testRecipient.trim()}.`;
      } else {
        error = `Delivery failed: ${result.error ?? "unknown error"}`;
      }
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not send the test email";
    } finally {
      testing = false;
    }
  }

  function sourceLabel(source: SmtpSettings["source"]): string {
    if (source === "database") return "Saved in SoloOps";
    if (source === "environment") return "From .env";
    return "Not configured";
  }
</script>

<svelte:head>
  <title>SMTP delivery · SoloOps</title>
</svelte:head>

<div class="p-6">
  <PageHeader
    crumbs={[{ label: "Settings" }, { label: "SMTP delivery" }]}
    title="SMTP delivery"
    subtitle="Outbound email relay for public IP notifications, password rotation and test messages."
  />

  {#if loading}
    <div class="mt-6 grid gap-4 lg:grid-cols-2">
      <div class="h-64 animate-pulse rounded-lg bg-ink-800/70"></div>
      <div class="h-64 animate-pulse rounded-lg bg-ink-800/70"></div>
    </div>
  {:else if settings}
    <div class="mt-6 grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_320px]">
      <!-- Left: configuration form -->
      <form class="panel space-y-4 p-5" onsubmit={save}>
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <span class="block text-sm font-medium text-foreground">Relay configuration</span>
            <span class="mt-1 block text-sm text-muted-foreground">
              Credentials are stored server-side and never sent back to the browser.
            </span>
          </div>
          <Badge variant={settings.configured ? "secondary" : "outline"}>
            {sourceLabel(settings.source)}
          </Badge>
        </div>

        <div class="grid gap-4 sm:grid-cols-[minmax(0,1fr)_120px]">
          <label class="block">
            <span class="mb-2 block text-sm text-foreground">Host</span>
            <Input bind:value={host} placeholder="smtp.example.com" required />
          </label>
          <label class="block">
            <span class="mb-2 block text-sm text-foreground">Port</span>
            <Input bind:value={port} type="number" min="1" max="65535" required />
          </label>
        </div>

        <div class="grid gap-4 sm:grid-cols-[160px_minmax(0,1fr)]">
          <label class="block">
            <span class="mb-2 block text-sm text-foreground">Encryption</span>
            <select
              bind:value={security}
              class="h-10 w-full rounded-md border border-edge bg-ink-900 px-3 text-sm text-foreground outline-none focus-visible:ring-2 focus-visible:ring-mint-400/40"
            >
              <option value="tls">TLS (465)</option>
              <option value="starttls">STARTTLS (587)</option>
            </select>
          </label>
          <label class="block">
            <span class="mb-2 block text-sm text-foreground">Sender (From)</span>
            <Input bind:value={from} placeholder="SoloOps <soloops@example.com>" required />
          </label>
        </div>

        <div class="grid gap-4 sm:grid-cols-2">
          <label class="block">
            <span class="mb-2 block text-sm text-foreground">Username</span>
            <Input
              bind:value={username}
              placeholder="Leave empty for relay without login"
              autocomplete="off"
            />
          </label>
          <label class="block">
            <span class="mb-2 block text-sm text-foreground">
              Password
              {#if settings.passwordConfigured}
                <span class="ml-1 text-2xs font-normal text-muted-foreground/70">
                  (saved — leave blank to keep)
                </span>
              {/if}
            </span>
            <Input
              bind:value={password}
              type="password"
              placeholder={settings.passwordConfigured ? "••••••••" : "SMTP password or auth code"}
              autocomplete="new-password"
            />
          </label>
        </div>

        {#if error}<p class="text-sm text-red-300">{error}</p>{/if}
        {#if message}<p class="text-sm text-mint-400">{message}</p>{/if}

        <div class="flex flex-wrap items-center gap-3 pt-1">
          <Button type="submit" disabled={saving}>{saving ? "Saving…" : "Save settings"}</Button>
          {#if settings.source === "database"}
            <Button
              type="button"
              variant="outline"
              disabled={clearing}
              onclick={clearSettings}>Remove saved settings</Button
            >
          {/if}
        </div>
      </form>

      <!-- Right: test panel -->
      <div class="panel space-y-4 p-5">
        <div>
          <span class="block text-sm font-medium text-foreground">Send a test email</span>
          <span class="mt-1 block text-sm text-muted-foreground">
            Verify the saved relay by delivering one message.
          </span>
        </div>
        <label class="block">
          <span class="mb-2 block text-sm text-foreground">Recipient</span>
          <Input
            bind:value={testRecipient}
            type="email"
            placeholder="you@example.com"
            disabled={!settings.configured}
          />
        </label>
        <Button
          type="button"
          variant="outline"
          disabled={!settings.configured || testing || !testRecipient.trim()}
          onclick={sendTest}
        >
          {#if testing}
            <span class="inline-flex items-center gap-2"><Icon name="clock" size={12} /> Sending…</span>
          {:else}
            <span class="inline-flex items-center gap-2"><Icon name="play" size={12} /> Send test</span>
          {/if}
        </Button>
        {#if !settings.configured}
          <p class="text-2xs leading-relaxed text-muted-foreground/70">
            Save a working relay configuration first; the button unlocks once SoloOps can build a
            mailer from the current settings.
          </p>
        {/if}
      </div>
    </div>
  {/if}
</div>
