<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { api, ApiClientError } from "$lib/api";
  import type { SshAccessReport, SshAuthorizedKeyEntry } from "$lib/contracts";
  import { formatDateTime, relativeTime } from "$lib/time";
  import Icon from "$lib/components/Icon.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { Button } from "$lib/components/ui/button";

  type AuditedKey = SshAuthorizedKeyEntry & { file: string };
  type MachineGroup = { name: string | null; keys: AuditedKey[] };

  let report = $state<SshAccessReport | null>(null);
  let loading = $state(true);
  let refreshing = $state(false);
  let error = $state("");

  onMount(load);

  async function load() {
    try {
      report = await api<SshAccessReport>("/api/settings/ssh-access");
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) return goto("/login");
      error = cause instanceof Error ? cause.message : "Could not load the SSH access report";
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  const machineGroups = $derived.by<MachineGroup[]>(() => {
    if (!report) return [];
    const groups: MachineGroup[] = report.machines.map((machine) => ({
      name: machine.name,
      keys: []
    }));
    const attach = (audited: AuditedKey) => {
      const group = groups.find((candidate) => candidate.name === audited.machine);
      if (group) {
        group.keys.push(audited);
      } else {
        groups.push({ name: audited.machine, keys: [audited] });
      }
    };
    for (const file of report.files) {
      for (const entry of file.entries) {
        if (entry.valid) attach({ ...entry, file: file.path });
      }
    }
    return groups;
  });

  const invalidEntries = $derived.by<AuditedKey[]>(() => {
    if (!report) return [];
    const rows: AuditedKey[] = [];
    for (const file of report.files) {
      for (const entry of file.entries) {
        if (!entry.valid) rows.push({ ...entry, file: file.path });
      }
    }
    return rows;
  });

  const restrictedKeys = $derived(
    machineGroups.reduce(
      (count, group) => count + group.keys.filter((key) => key.fromPatterns).length,
      0
    )
  );

  function plainOptions(entry: SshAuthorizedKeyEntry): string[] {
    return entry.options.filter(
      (option) => !option.startsWith("from=") && !option.startsWith("command=")
    );
  }

  function scanTitle(): string {
    return report ? formatDateTime(report.scannedAt) : "Never";
  }

  function scanTime(): string {
    return report ? relativeTime(report.scannedAt) : "Never";
  }
</script>

<svelte:head>
  <title>Security · SoloOps</title>
</svelte:head>

<div class="p-6">
  <PageHeader
    crumbs={[{ label: "Settings" }, { label: "Security" }]}
    title="SSH access audit"
    subtitle="Machines holding a public key that may log in to this host without a password."
  >
    {#snippet actions()}
      <Button variant="outline" type="button" onclick={load} disabled={loading || refreshing}>
        <Icon name="refresh" size={12} />
        {refreshing ? "Scanning…" : "Rescan"}
      </Button>
    {/snippet}
  </PageHeader>

  {#if loading}
    <div class="mt-6 grid gap-4 lg:grid-cols-2">
      <div class="h-44 animate-pulse rounded-lg bg-ink-800/70"></div>
      <div class="h-44 animate-pulse rounded-lg bg-ink-800/70"></div>
    </div>
  {:else if report}
    <!-- Summary strip -->
    <section class="command-strip mt-6">
      <div class="px-3 py-2.5">
        <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
          Machines
        </p>
        <p class="mt-0.5 text-sm text-foreground">
          {report.machines.filter((machine) => machine.name !== null).length}
        </p>
      </div>
      <div class="px-3 py-2.5">
        <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
          Authorized keys
        </p>
        <p class="mt-0.5 text-sm text-foreground">{report.totalKeys}</p>
      </div>
      <div class="px-3 py-2.5">
        <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
          Host-restricted
        </p>
        <p class="mt-0.5 text-sm text-foreground">{restrictedKeys}</p>
      </div>
      <div class="px-3 py-2.5">
        <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
          Invalid lines
        </p>
        <p class="mt-0.5 text-sm text-foreground">{report.invalidLines}</p>
      </div>
      <div class="px-3 py-2.5">
        <p class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
          Last scan
        </p>
        <p class="mt-0.5 text-sm text-muted-foreground" title={scanTitle()}>{scanTime()}</p>
      </div>
    </section>

    <div class="mt-4 grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_320px]">
      <!-- Left: machines that may connect without a password -->
      <div class="min-w-0 space-y-4">
        {#if report.totalKeys === 0}
          <section class="panel p-5">
            <div class="flex items-center gap-2 text-mint-400">
              <Icon name="shield" size={14} />
              <h2 class="text-sm font-semibold tracking-tight text-foreground">
                No authorized public keys
              </h2>
            </div>
            <p class="mt-2 text-sm leading-relaxed text-muted-foreground">
              No machine holds a key that may connect to this host without a password.
            </p>
          </section>
        {:else}
          {#each machineGroups as group (group.name ?? "__unknown__")}
            <section class="panel p-5">
              <div class="flex items-center justify-between gap-2">
                <h2
                  class="flex min-w-0 items-center gap-2 text-sm font-semibold tracking-tight text-foreground"
                >
                  <Icon name="terminal" size={14} class="shrink-0 text-muted-foreground/65" />
                  <span class="truncate">{group.name ?? "Unknown source"}</span>
                </h2>
                <span class="shrink-0 font-mono text-2xs text-muted-foreground/65">
                  {group.keys.length}
                  {group.keys.length === 1 ? "key" : "keys"}
                </span>
              </div>

              <ul class="mt-3 space-y-2">
                {#each group.keys as key (key.fingerprint)}
                  <li class="rounded-lg border border-edge px-3 py-2.5">
                    <div class="flex items-center justify-between gap-2">
                      <span class="flex min-w-0 items-center gap-2">
                        <Icon name="key" size={12} class="shrink-0 text-muted-foreground/65" />
                        <span class="truncate text-sm text-foreground">{key.keyType}</span>
                        {#if key.keyBits !== null}
                          <span
                            class="shrink-0 rounded border border-edge px-1.5 py-0.5 text-2xs text-muted-foreground/65"
                          >
                            {key.keyBits} bits
                          </span>
                        {/if}
                      </span>
                      <span
                        class="shrink-0 font-mono text-2xs text-muted-foreground/65"
                        title={key.file}
                      >
                        line {key.line}
                      </span>
                    </div>
                    <p
                      class="mt-1 truncate font-mono text-xs text-muted-foreground"
                      title={key.fingerprint ?? ""}
                    >
                      {key.fingerprint}
                    </p>
                    <div class="mt-1.5 flex flex-wrap items-center gap-1.5">
                      {#each key.fromPatterns ?? [] as pattern (pattern)}
                        <span
                          class="rounded border border-mint-400/30 bg-mint-400/10 px-1.5 py-0.5 text-2xs text-mint-300"
                          title="Key only accepts connections from matching hosts"
                        >
                          from: {pattern}
                        </span>
                      {/each}
                      {#if key.forcedCommand}
                        <span
                          class="max-w-full truncate rounded border border-edge px-1.5 py-0.5 text-2xs text-muted-foreground"
                          title={key.forcedCommand}
                        >
                          command: {key.forcedCommand}
                        </span>
                      {/if}
                      {#each plainOptions(key) as option (option)}
                        <span
                          class="rounded border border-edge px-1.5 py-0.5 text-2xs text-muted-foreground"
                        >
                          {option}
                        </span>
                      {/each}
                      {#if key.comment && key.machine === null}
                        <span
                          class="max-w-full truncate text-2xs text-muted-foreground/65"
                          title={key.comment}
                        >
                          {key.comment}
                        </span>
                      {/if}
                    </div>
                  </li>
                {/each}
              </ul>
            </section>
          {/each}
        {/if}

        {#if invalidEntries.length > 0}
          <section class="rounded-lg border border-amber-400/30 bg-amber-400/10 p-5">
            <div class="flex items-center gap-2 text-amber-300">
              <Icon name="alert" size={14} />
              <h2 class="text-sm font-semibold tracking-tight text-foreground">
                Unreadable lines ({invalidEntries.length})
              </h2>
            </div>
            <ul class="mt-3 space-y-1.5">
              {#each invalidEntries as entry (entry.file + entry.line)}
                <li class="flex items-start justify-between gap-2 text-sm">
                  <span class="min-w-0 truncate text-amber-300">{entry.error}</span>
                  <span
                    class="shrink-0 font-mono text-2xs text-muted-foreground/65"
                    title={entry.file}
                  >
                    line {entry.line}
                  </span>
                </li>
              {/each}
            </ul>
          </section>
        {/if}
      </div>

      <!-- Right: scan context rail -->
      <div class="space-y-4">
        <section class="panel p-5">
          <div class="flex items-center gap-2">
            <Icon name="server" size={14} class="text-muted-foreground/65" />
            <h2 class="text-sm font-semibold tracking-tight text-foreground">Scan context</h2>
          </div>
          <dl class="mt-3 space-y-2 text-sm">
            <div class="flex items-baseline justify-between gap-2">
              <dt class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
                Account
              </dt>
              <dd class="truncate font-mono text-muted-foreground" title={report.user ?? "Unknown"}>
                {report.user ?? "Unknown"}
              </dd>
            </div>
            <div class="flex items-baseline justify-between gap-2">
              <dt class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
                Platform
              </dt>
              <dd class="text-muted-foreground">{report.platform}</dd>
            </div>
            <div class="flex items-baseline justify-between gap-2">
              <dt class="text-2xs font-medium uppercase tracking-[0.12em] text-muted-foreground/65">
                Home
              </dt>
              <dd
                class="min-w-0 truncate font-mono text-muted-foreground"
                title={report.homeDir ?? "Unknown"}
              >
                {report.homeDir ?? "Unknown"}
              </dd>
            </div>
          </dl>
        </section>

        <section class="panel p-5">
          <h2 class="text-sm font-semibold tracking-tight text-foreground">Scanned files</h2>
          <ul class="mt-3 space-y-2">
            {#each report.files as file (file.path)}
              <li class="rounded-lg border border-edge px-3 py-2">
                <div class="flex items-start justify-between gap-2">
                  <span class="flex min-w-0 items-center gap-2" title="{file.path} ({file.role})">
                    <Icon name="file" size={12} class="shrink-0 text-muted-foreground/65" />
                    <span class="truncate font-mono text-xs text-foreground">{file.path}</span>
                  </span>
                  {#if file.exists}
                    <span
                      class={`shrink-0 rounded px-1.5 py-0.5 text-2xs ${
                        file.entries.length > 0
                          ? "border border-mint-400/30 bg-mint-400/10 text-mint-300"
                          : "border border-edge text-muted-foreground/65"
                      }`}
                    >
                      {file.entries.length}
                      {file.entries.length === 1 ? "entry" : "entries"}
                    </span>
                  {:else}
                    <span
                      class="shrink-0 rounded border border-edge px-1.5 py-0.5 text-2xs text-muted-foreground/65"
                    >
                      missing
                    </span>
                  {/if}
                </div>
                {#if file.error}
                  <p class="mt-1 text-2xs text-red-300">{file.error}</p>
                {/if}
              </li>
            {/each}
          </ul>
        </section>

        <section class="panel p-5">
          <div class="flex items-center gap-2">
            <Icon name="info" size={14} class="text-muted-foreground/65" />
            <h2 class="text-sm font-semibold tracking-tight text-foreground">How to read this</h2>
          </div>
          <p class="mt-2 text-sm leading-relaxed text-muted-foreground/65">
            Machine names are inferred from key comments (<span class="font-mono">user@host</span>)
            and can be arbitrary; the fingerprint identifies the real key. A
            <span class="font-mono">from=</span> option restricts which hosts may use the key, and
            <span class="font-mono">command=</span> limits the login to a single forced command. Editing
            these files is out of scope — audit only.
          </p>
        </section>
      </div>
    </div>
  {:else if error}
    <p class="mt-6 text-sm text-red-300">{error}</p>
  {/if}
</div>
