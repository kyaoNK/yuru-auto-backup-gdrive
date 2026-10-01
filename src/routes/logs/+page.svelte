<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api } from "$lib/api";
  import { createAsyncScope } from "$lib/async-scope";
  const scope = createAsyncScope();
  let lines = $state<string[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let subscriptionError = $state<string | null>(null);
  let filter = $state<"all" | "info" | "warn" | "error">("all");

  async function refresh() {
    const current = scope.ticket("logs");
    loading = true;
    try {
      const recent = await api.listRecentLogs(500);
      if (current()) { lines = [...recent].reverse(); error = null; }
    } catch (e) { if (current()) error = String(e); }
    finally { if (current()) loading = false; }
  }

  onMount(() => {
    void Promise.all(["status-changed", "job-finished", "error-occurred"].map(event =>
      scope.own(listen(event, () => { if (scope.active) void refresh(); }), e => { subscriptionError = String(e); })
    )).then(() => { if (scope.active) void refresh(); });
    return () => scope.dispose();
  });

  const level = (line: string) => /^\[[^\]]+\] \[(INFO|WARN|ERROR)\]/.exec(line)?.[1];
  const filtered = $derived(filter === "all" ? lines : lines.filter(line => level(line) === filter.toUpperCase()));
  function levelClass(line: string): string {
    if (level(line) === "ERROR") return "text-red-600 dark:text-red-400";
    if (level(line) === "WARN") return "text-amber-600 dark:text-amber-400";
    return "text-slate-700 dark:text-slate-300";
  }
</script>

<section class="space-y-4">
  {#if subscriptionError}<p role="alert" class="text-sm text-red-700 dark:text-red-300">自動更新を受信できません。再読み込みをご利用ください：{subscriptionError}</p>{/if}
  <div class="flex flex-wrap gap-2 items-baseline justify-between">
    <h2 class="text-xl font-semibold">ログ（新しい順）</h2>
    <div class="flex flex-wrap gap-2 text-sm">
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
          aria-pressed={filter === opt.value}
          onclick={() => (filter = opt.value)}
        >{opt.label}</button>
      {/each}
      <button
        class="px-2 py-1 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700"
        onclick={refresh}
      >再読み込み</button>
    </div>
  </div>

  {#if error}<p role="alert" class="break-words text-sm text-red-700 dark:text-red-300">ログを取得できませんでした：{error}</p>{/if}
  {#if loading}
    <p class="text-sm text-slate-500">読み込み中…</p>
  {:else if error}
    <p class="text-sm">再読み込みをお試しください。</p>
  {:else if filtered.length === 0}
    <p class="text-sm text-slate-500">{filter === "all" ? "ログはまだありません。" : "この条件に一致するログはありません。"}</p>
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
