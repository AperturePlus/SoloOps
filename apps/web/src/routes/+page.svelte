<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { api } from "$lib/api";

  onMount(async () => {
    try {
      await api("/api/auth/session");
      await goto("/tasks");
    } catch {
      await goto("/login");
    }
  });
</script>

<p class="text-sm text-slate-400">Loading SoloOps…</p>

