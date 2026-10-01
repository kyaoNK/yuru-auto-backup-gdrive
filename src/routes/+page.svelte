<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api } from "$lib/api";
  import { createAsyncScope } from "$lib/async-scope";
  import { formatDateTime } from "$lib/format";
  import type { DeletionPreview, JobSummary, Status } from "$lib/types";

  const scope = createAsyncScope();
  let status = $state<Status | null>(null);
  let loading = $state(true);
  let running = $state(false);
  let toast = $state<string | null>(null);
  let refreshError = $state<string | null>(null);
  let preview = $state<DeletionPreview | null>(null);
  let previewLoading = $state(false);
  let previewError = $state<string | null>(null);

  function invalidatePreview() {
    scope.ticket("preview");
    if (preview || previewLoading) previewError = "状態が変わったため、削除予定を再確認してください。";
    preview = null;
    previewLoading = false;
  }

  async function refresh() {
    const current = scope.ticket("status");
    try {
      const result = await api.getStatus();
      if (current()) { status = result; refreshError = null; }
    } catch (e) {
      if (current()) refreshError = String(e);
    } finally {
      if (current()) loading = false;
    }
  }

  async function handleRunNow() {
    if (running) return;
    invalidatePreview();
    running = true;
    try {
      const accepted = await api.runNow();
      if (scope.active) toast = accepted ? "バックアップの実行を受け付けました" : "すでに実行中、または受付済みです";
    } catch (e) {
      if (scope.active) toast = `実行に失敗: ${e}`;
    } finally {
      if (scope.active) { running = false; void refresh(); }
    }
  }

  async function handlePreview() {
    if (previewLoading) return;
    const current = scope.ticket("preview");
    previewLoading = true;
    preview = null;
    previewError = null;
    try {
      const result = await api.previewDeletions();
      if (current()) preview = result;
    } catch (e) {
      if (current()) previewError = `削除予定を確認できませんでした: ${e}`;
    } finally {
      if (current()) previewLoading = false;
    }
  }

  onMount(() => {
    const failed = (error: unknown) => { toast = `通知の受信に失敗: ${error}`; };
    const changed = () => { if (scope.active) { invalidatePreview(); void refresh(); } };
    void Promise.all([
      scope.own(listen("status-changed", changed), failed),
      scope.own(listen<JobSummary>("job-finished", (e) => {
        if (!scope.active) return;
        toast = `完了: コピー ${e.payload.copied} 件 / エラー ${e.payload.errors} 件`;
        changed();
      }), failed),
      scope.own(listen<string>("error-occurred", (e) => {
        if (!scope.active) return;
        toast = `エラー: ${e.payload}`;
        changed();
      }), failed),
    ]).then(() => { if (scope.active) void refresh(); });
    return () => scope.dispose();
  });

  const statusLabel = $derived.by(() => {
    if (refreshError) return "状態を取得できません";
    if (!status) return "読み込み中";
    if (status.running) return "実行中";
    if (status.serviceError || status.lastError) return "エラー：詳細を確認してください";
    if (!status.source || !status.destination) return "未設定";
    if (status.lastSummary && status.lastSummary.errors > 0) return "エラー：前回の処理で一部失敗";
    return status.nextRunAt ? `次回 ${formatDateTime(status.nextRunAt)}` : "予定を確認中";
  });
</script>

<section class="space-y-6">
  <div class="flex items-baseline justify-between">
    <h2 class="text-xl font-semibold">ダッシュボード</h2>
    <span class="text-sm text-slate-500 dark:text-slate-400">{statusLabel}</span>
  </div>

  {#if refreshError}
    <div role="alert" class="space-y-2 break-words text-sm text-red-700 dark:text-red-300">
      <p>状態を読み込めませんでした：{refreshError}</p>
      <button class="underline" onclick={refresh}>再読み込み</button>
      <a class="ml-3 underline" href="/settings">設定・保存先を確認</a>
    </div>
  {/if}
  {#if loading}
    <p class="text-sm text-slate-500">読み込み中…</p>
  {:else if status}
    {#if status.serviceError}
      <p role="alert" class="break-words text-sm text-red-700 dark:text-red-300">{status.serviceError}</p>
    {/if}
    {#if status.lastError}
      <div role="alert" class="rounded-lg border border-red-300 bg-red-50 p-4 text-red-800 dark:border-red-700 dark:bg-red-950 dark:text-red-200">
        <p class="font-semibold">直近のバックアップに失敗しました</p>
        <p class="mt-1 text-sm">失敗日時：{formatDateTime(status.lastError.at)}</p>
        <p class="mt-1 break-words text-sm">{status.lastError.message}</p>
        <a href="/logs" class="mt-2 inline-block text-sm underline">ログで詳細を確認</a>
      </div>
    {:else if status.lastSummary && status.lastSummary.errors > 0}
      <div role="alert" class="rounded-lg border border-red-300 bg-red-50 p-4 text-red-800 dark:border-red-700 dark:bg-red-950 dark:text-red-200">
        <p class="font-semibold">前回のバックアップ処理で {status.lastSummary.errors} 件のエラーが発生しました</p>
        <a href="/logs" class="mt-2 inline-block text-sm underline">ログで詳細を確認</a>
      </div>
    {/if}
    <div class="grid gap-4 sm:grid-cols-2">
      <div class="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 p-4">
        <div class="text-xs uppercase tracking-wide text-slate-500">{status.lastError ? "前回完了した実行（直近の失敗とは別）" : "最終実行"}</div>
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

    <div class="flex flex-wrap gap-3">
      <button
        class="px-4 py-2 rounded-md bg-blue-600 text-white font-medium hover:bg-blue-700 disabled:opacity-50"
        onclick={handleRunNow}
        disabled={running || status.running || !!refreshError || !status.source || !status.destination}
      >
        {status.running ? "実行中…" : "今すぐ実行"}
      </button>
      <button
        class="min-h-11 px-4 py-2 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700 disabled:opacity-50"
        onclick={handlePreview}
        disabled={previewLoading || running || status.running || !!refreshError || !status.source || !status.destination}
        aria-busy={previewLoading}
      >
        {previewLoading ? "削除予定を確認中…" : "削除予定を確認"}
      </button>
      <a
        href="/settings"
        class="px-4 py-2 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700"
      >
        設定を開く
      </a>
    </div>
    <p class="text-sm text-slate-600 dark:text-slate-300">確認だけではコピー・削除しません。定時／手動実行では期限切れの管理済みバックアップを自動削除し、Drive 側にも同期されます。</p>
    {#if previewError}
      <p role="alert" class="break-words text-sm text-red-700 dark:text-red-300">{previewError}</p>
    {/if}
    {#if preview}
      <section aria-label="削除予定" class="space-y-3 rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-700 dark:bg-slate-800">
        <h3 class="font-semibold" aria-live="polite">削除予定：{preview.candidates.length} 件</h3>
        <p class="text-sm text-slate-600 dark:text-slate-300">確認日時：{formatDateTime(preview.checkedAt)}。保存済み設定での確認結果です。実行時に再判定するため、結果は変わることがあります。</p>
        {#if preview.errors.length > 0}
          <div role="alert" class="text-sm text-red-700 dark:text-red-300">
            <p>確認できなかった項目が {preview.errors.length} 件あります。一覧は完全ではありません。</p>
            <ul class="mt-2 max-h-48 overflow-y-auto space-y-2">
              {#each preview.errors as [path, message]}
                <li class="break-all">{path}：{message}</li>
              {/each}
            </ul>
          </div>
        {/if}
        {#if preview.candidates.length > 0}
          <ul class="max-h-64 overflow-y-auto space-y-2 text-sm">
            {#each preview.candidates as path}<li class="break-all">{path}</li>{/each}
          </ul>
        {:else if preview.errors.length === 0}
          <p class="text-sm">現在、削除予定のバックアップはありません。</p>
        {/if}
        {#if preview.retained.length > 0}
          <details class="text-sm">
            <summary class="cursor-pointer py-2">削除しないバックアップ：{preview.retained.length} 件（未管理・外部変更など）</summary>
            <ul class="mt-2 max-h-64 overflow-y-auto space-y-2">
              {#each preview.retained as path}<li class="break-all">{path}</li>{/each}
            </ul>
          </details>
        {/if}
      </section>
    {/if}
  {/if}

  {#if toast}
    <div role="status" class="break-words rounded-md bg-slate-900 text-white px-4 py-2 text-sm">{toast}</div>
  {/if}
</section>
