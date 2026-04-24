<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { api } from "$lib/api";
  import { formatDateTime } from "$lib/format";
  import type { Status } from "$lib/types";

  let status = $state<Status | null>(null);
  let loading = $state(true);
  let running = $state(false);
  let toast = $state<string | null>(null);
  let unlistens: UnlistenFn[] = [];

  async function refresh() {
    try {
      status = await api.getStatus();
    } catch (e) {
      toast = String(e);
    } finally {
      loading = false;
    }
  }

  async function handleRunNow() {
    running = true;
    try {
      await api.runNow();
      toast = "バックアップを開始しました";
    } catch (e) {
      toast = `実行に失敗: ${e}`;
    } finally {
      running = false;
    }
  }

  onMount(async () => {
    await refresh();
    unlistens.push(await listen("status-changed", () => refresh()));
    unlistens.push(
      await listen<{ copied: number; errors: number }>("job-finished", (e) => {
        toast = `完了: コピー ${e.payload.copied} 件 / エラー ${e.payload.errors} 件`;
        refresh();
      }),
    );
    unlistens.push(
      await listen<string>("error-occurred", (e) => {
        toast = `エラー: ${e.payload}`;
        refresh();
      }),
    );
  });

  onDestroy(() => {
    unlistens.forEach((fn) => fn());
  });

  const statusLabel = $derived.by(() => {
    if (!status) return "読み込み中";
    if (status.running) return "⏳ 実行中";
    if (!status.source || !status.destination) return "⚠ 未設定";
    return `● 次回 ${status.scheduleTime}`;
  });
</script>

<section class="space-y-6">
  <div class="flex items-baseline justify-between">
    <h2 class="text-xl font-semibold">ダッシュボード</h2>
    <span class="text-sm text-slate-500 dark:text-slate-400">{statusLabel}</span>
  </div>

  {#if loading}
    <p class="text-sm text-slate-500">読み込み中…</p>
  {:else if status}
    <div class="grid gap-4 sm:grid-cols-2">
      <div class="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 p-4">
        <div class="text-xs uppercase tracking-wide text-slate-500">最終実行</div>
        <div class="mt-1 text-lg font-medium">{formatDateTime(status.lastRunAt)}</div>
        {#if status.lastSummary}
          <div class="mt-1 text-sm text-slate-600 dark:text-slate-300">
            コピー {status.lastSummary.copied} 件 / エラー {status.lastSummary.errors} 件
          </div>
        {/if}
      </div>

      <div class="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 p-4">
        <div class="text-xs uppercase tracking-wide text-slate-500">次回実行</div>
        <div class="mt-1 text-lg font-medium">{formatDateTime(status.nextRunAt)}</div>
        <div class="mt-1 text-sm text-slate-600 dark:text-slate-300">
          毎日 {status.scheduleTime}
        </div>
      </div>
    </div>

    <div class="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 p-4 space-y-2">
      <div>
        <div class="text-xs uppercase tracking-wide text-slate-500">監視元</div>
        <div class="text-sm break-all">{status.source ?? "— 未設定"}</div>
      </div>
      <div>
        <div class="text-xs uppercase tracking-wide text-slate-500">出力先</div>
        <div class="text-sm break-all">{status.destination ?? "— 未設定"}</div>
      </div>
    </div>

    <div class="flex gap-3">
      <button
        class="px-4 py-2 rounded-md bg-blue-600 text-white font-medium hover:bg-blue-700 disabled:opacity-50"
        onclick={handleRunNow}
        disabled={running || status.running || !status.source || !status.destination}
      >
        {status.running ? "実行中…" : "今すぐ実行"}
      </button>
      <a
        href="/settings"
        class="px-4 py-2 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700"
      >
        設定を開く
      </a>
    </div>
  {/if}

  {#if toast}
    <div class="rounded-md bg-slate-900 text-white px-4 py-2 text-sm">{toast}</div>
  {/if}
</section>
