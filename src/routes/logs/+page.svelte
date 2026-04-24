<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { api } from "$lib/api";

  let lines = $state<string[]>([]);
  let loading = $state(true);
  let filter = $state<"all" | "info" | "warn" | "error">("all");
  let unlistens: UnlistenFn[] = [];

  async function refresh() {
    try {
      lines = await api.listRecentLogs(500);
    } finally {
      loading = false;
    }
  }

  onMount(async () => {
    await refresh();
    unlistens.push(await listen("status-changed", () => refresh()));
    unlistens.push(await listen("job-finished", () => refresh()));
    unlistens.push(await listen("error-occurred", () => refresh()));
  });

  onDestroy(() => unlistens.forEach((fn) => fn()));

  const filtered = $derived.by(() => {
    if (filter === "all") return lines;
    const tag = filter === "info" ? "[INFO]" : filter === "warn" ? "[WARN]" : "[ERROR]";
    return lines.filter((l) => l.includes(tag));
  });

  function levelClass(line: string): string {
    if (line.includes("[ERROR]")) return "text-red-600 dark:text-red-400";
    if (line.includes("[WARN]")) return "text-amber-600 dark:text-amber-400";
    return "text-slate-700 dark:text-slate-300";
  }
</script>

<section class="space-y-4">
  <div class="flex items-baseline justify-between">
    <h2 class="text-xl font-semibold">ログ</h2>
    <div class="flex gap-2 text-sm">
      {#each [
        { value: "all", label: "すべて" },
        { value: "info", label: "情報" },
        { value: "warn", label: "警告" },
        { value: "error", label: "エラー" },
      ] as const as opt}
        <button
          class="px-2 py-1 rounded-md border"
          class:bg-slate-200={filter === opt.value}
          class:dark:bg-slate-700={filter === opt.value}
          class:border-slate-300={filter !== opt.value}
          class:dark:border-slate-600={filter !== opt.value}
          onclick={() => (filter = opt.value)}
        >{opt.label}</button>
      {/each}
      <button
        class="px-2 py-1 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700"
        onclick={refresh}
      >再読み込み</button>
    </div>
  </div>

  {#if loading}
    <p class="text-sm text-slate-500">読み込み中…</p>
  {:else if filtered.length === 0}
    <p class="text-sm text-slate-500">ログはまだありません。</p>
  {:else}
    <div class="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800">
      <ul class="divide-y divide-slate-100 dark:divide-slate-700 text-xs font-mono max-h-[60vh] overflow-y-auto">
        {#each filtered as line}
          <li class="px-3 py-1.5 break-all {levelClass(line)}">{line}</li>
        {/each}
      </ul>
    </div>
  {/if}
</section>
