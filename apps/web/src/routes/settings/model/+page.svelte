<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { api, ApiClientError } from "$lib/api";
  import type {
    ModelSettings,
    TestModelSettingsResponse,
    UpdateModelSettingsRequest
  } from "$lib/contracts";
  import Icon from "$lib/components/Icon.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";

  let settings = $state<ModelSettings | null>(null);
  let baseUrl = $state("");
  let modelName = $state("");
  let apiKey = $state("");
  let loading = $state(true);
  let saving = $state(false);
  let testing = $state(false);
  let clearing = $state(false);
  let error = $state("");
  let message = $state("");

  onMount(load);

  async function load() {
    try {
      settings = await api<ModelSettings>("/api/settings/model");
      if (settings.baseUrl) baseUrl = settings.baseUrl;
      if (settings.modelName) modelName = settings.modelName;
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not load model API settings";
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
      const body: UpdateModelSettingsRequest = {
        baseUrl: baseUrl.trim(),
        modelName: modelName.trim()
      };
      // Only send the API key when the owner typed a new one; an explicit
      // empty value clears the stored credential (local endpoints).
      if (apiKey !== "") body.apiKey = apiKey;
      settings = await api<ModelSettings>("/api/settings/model", {
        method: "PUT",
        body: JSON.stringify(body)
      });
      if (settings.baseUrl) baseUrl = settings.baseUrl;
      if (settings.modelName) modelName = settings.modelName;
      apiKey = "";
      message = "Model API settings saved. The worker picks them up on its next poll.";
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not save model API settings";
    } finally {
      saving = false;
    }
  }

  async function clearSettings() {
    clearing = true;
    error = "";
    message = "";
    try {
      await api<void>("/api/settings/model", { method: "DELETE" });
      settings = await api<ModelSettings>("/api/settings/model");
      if (settings.baseUrl) baseUrl = settings.baseUrl;
      else baseUrl = "";
      if (settings.modelName) modelName = settings.modelName;
      else modelName = "";
      apiKey = "";
      message = "Saved settings removed; the agent runtime falls back to .env configuration.";
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not clear model API settings";
    } finally {
      clearing = false;
    }
  }

  async function sendTest() {
    testing = true;
    error = "";
    message = "";
    try {
      const result = await api<TestModelSettingsResponse>("/api/settings/model/test", {
        method: "POST"
      });
      if (result.responded) {
        message = "The model answered the test request.";
      } else {
        error = `Test request failed: ${result.error ?? "unknown error"}`;
      }
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not send the test request";
    } finally {
      testing = false;
    }
  }

  function sourceLabel(source: ModelSettings["source"]): string {
    if (source === "database") return "Saved in SoloOps";
    if (source === "environment") return "From .env";
    return "Not configured";
  }
</script>

<svelte:head>
  <title>Model API · SoloOps</title>
</svelte:head>

<div class="p-6">
  <PageHeader
    crumbs={[{ label: "Settings" }, { label: "Model API" }]}
    title="Model API"
    subtitle="LLM endpoint, model name and API key used by the agent runtime. Saved settings override the .env configuration without a restart."
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
            <span class="block text-sm font-medium text-foreground">Endpoint configuration</span>
            <span class="mt-1 block text-sm text-muted-foreground">
              The API key is stored server-side and never sent back to the browser.
            </span>
          </div>
          <Badge variant={settings.configured ? "secondary" : "outline"}>
            {sourceLabel(settings.source)}
          </Badge>
        </div>

        <label class="block">
          <span class="mb-2 block text-sm text-foreground">Base URL</span>
          <Input
            bind:value={baseUrl}
            placeholder="https://api.openai.com/v1"
            autocomplete="off"
            required
          />
          <span class="mt-1.5 block text-2xs text-muted-foreground/70">
            OpenAI-compatible Chat Completions endpoint; SoloOps appends /chat/completions.
          </span>
        </label>

        <div class="grid gap-4 sm:grid-cols-2">
          <label class="block">
            <span class="mb-2 block text-sm text-foreground">Model name</span>
            <Input bind:value={modelName} placeholder="gpt-4o-mini" autocomplete="off" required />
          </label>
          <label class="block">
            <span class="mb-2 block text-sm text-foreground">
              API key
              {#if settings.apiKeyConfigured}
                <span class="ml-1 text-2xs font-normal text-muted-foreground/70">
                  (saved — leave blank to keep)
                </span>
              {/if}
            </span>
            <Input
              bind:value={apiKey}
              type="password"
              placeholder={settings.apiKeyConfigured
                ? "••••••••"
                : "sk-… (leave empty for endpoints without auth)"}
              autocomplete="new-password"
            />
          </label>
        </div>

        {#if error}<p class="text-sm text-red-300">{error}</p>{/if}
        {#if message}<p class="text-sm text-mint-400">{message}</p>{/if}

        <div class="flex flex-wrap items-center gap-3 pt-1">
          <Button type="submit" disabled={saving}>{saving ? "Saving…" : "Save settings"}</Button>
          {#if settings.source === "database"}
            <Button type="button" variant="outline" disabled={clearing} onclick={clearSettings}
              >Remove saved settings</Button
            >
          {/if}
        </div>
      </form>

      <!-- Right: test panel -->
      <div class="panel space-y-4 p-5">
        <div>
          <span class="block text-sm font-medium text-foreground">Send a test request</span>
          <span class="mt-1 block text-sm text-muted-foreground">
            Sends a minimal chat completion through the saved endpoint to verify the model and
            credential end to end.
          </span>
        </div>
        <Button
          type="button"
          variant="outline"
          disabled={!settings.configured || testing}
          onclick={sendTest}
        >
          {#if testing}
            <span class="inline-flex items-center gap-2"
              ><Icon name="clock" size={12} /> Testing…</span
            >
          {:else}
            <span class="inline-flex items-center gap-2"
              ><Icon name="play" size={12} /> Send test</span
            >
          {/if}
        </Button>
        {#if !settings.configured}
          <p class="text-2xs leading-relaxed text-muted-foreground/70">
            Save a working endpoint configuration first; the button unlocks once SoloOps can build a
            provider from the current settings.
          </p>
        {/if}
      </div>
    </div>
  {/if}
</div>
