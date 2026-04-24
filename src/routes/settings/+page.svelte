<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "$lib/api";
  import type { Config, DriveCandidate } from "$lib/types";

  let config = $state<Config | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let toast = $state<string | null>(null);
  let detecting = $state(false);
  let driveCandidates = $state<DriveCandidate[]>([]);
  let showCandidates = $state(false);

  onMount(async () => {
    try {
      config = await api.getConfig();
    } catch (e) {
      toast = String(e);
    } finally {
      loading = false;
    }
  });

  async function pickSource() {
    if (!config) return;
    const picked = await api.pickFolder(config.source ?? undefined);
    if (picked) config.source = picked;
  }

  async function pickDestination(startDir?: string) {
    if (!config) return;
    const picked = await api.pickFolder(startDir ?? config.destination ?? undefined);
    if (picked) {
      config.destination = picked;
      showCandidates = false;
    }
  }

  async function detectDrives() {
    detecting = true;
    try {
      driveCandidates = await api.detectDriveRoots();
      if (driveCandidates.length === 0) {
        toast = "Google Drive が見つかりません。起動を確認してから手動で選んでください。";
      } else {
        showCandidates = true;
      }
    } catch (e) {
      toast = String(e);
    } finally {
      detecting = false;
    }
  }

  async function save() {
    if (!config) return;
    saving = true;
    toast = null;
    try {
      await api.updateConfig(config);
      toast = "保存しました";
    } catch (e) {
      toast = `保存に失敗: ${e}`;
    } finally {
      saving = false;
    }
  }

  const scheduleValid = $derived.by(() => {
    if (!config) return false;
    return /^\d{2}:\d{2}$/.test(config.scheduleTime);
  });
</script>

<section class="space-y-6">
  <h2 class="text-xl font-semibold">設定</h2>

  {#if loading}
    <p class="text-sm text-slate-500">読み込み中…</p>
  {:else if config}
    <div class="space-y-5">
      <div class="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 p-4 space-y-2">
        <label class="block text-sm font-medium" for="source">監視元フォルダ</label>
        <div class="flex gap-2">
          <input
            id="source"
            type="text"
            readonly
            value={config.source ?? ""}
            placeholder="フォルダを選択してください"
            class="flex-1 rounded-md border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 px-3 py-2 text-sm"
          />
          <button
            class="px-3 py-2 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700 text-sm"
            onclick={pickSource}
          >選択…</button>
        </div>
      </div>

      <div class="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 p-4 space-y-3">
        <label class="block text-sm font-medium" for="destination">出力先フォルダ (Google Drive 同期)</label>
        <div class="flex gap-2">
          <input
            id="destination"
            type="text"
            readonly
            value={config.destination ?? ""}
            placeholder="Google Drive 同期配下のフォルダを選択"
            class="flex-1 rounded-md border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 px-3 py-2 text-sm"
          />
          <button
            class="px-3 py-2 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700 text-sm"
            onclick={() => pickDestination()}
          >選択…</button>
        </div>
        <div class="flex gap-2">
          <button
            class="text-sm text-blue-600 hover:underline disabled:opacity-50"
            onclick={detectDrives}
            disabled={detecting}
          >
            {detecting ? "検出中…" : "Google Drive を検出"}
          </button>
        </div>
        {#if showCandidates && driveCandidates.length > 0}
          <ul class="divide-y divide-slate-200 dark:divide-slate-700 border border-slate-200 dark:border-slate-700 rounded-md">
            {#each driveCandidates as c}
              <li class="px-3 py-2 flex items-center justify-between gap-2">
                <div class="min-w-0">
                  <div class="text-sm font-medium truncate">{c.label}</div>
                  <div class="text-xs text-slate-500 truncate">{c.path}</div>
                </div>
                <button
                  class="text-sm px-2 py-1 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700"
                  onclick={() => pickDestination(c.path)}
                >ここを起点に選択</button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>

      <div class="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 p-4 grid gap-4 sm:grid-cols-2">
        <div>
          <label class="block text-sm font-medium" for="schedule">実行時刻</label>
          <input
            id="schedule"
            type="time"
            bind:value={config.scheduleTime}
            class="mt-1 w-full rounded-md border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 px-3 py-2 text-sm"
          />
          {#if !scheduleValid}
            <p class="mt-1 text-xs text-red-600">HH:MM 形式で指定してください</p>
          {/if}
        </div>
        <label class="flex items-center gap-2 pt-6">
          <input type="checkbox" bind:checked={config.autoStart} class="rounded" />
          <span class="text-sm">Windows ログオン時に自動起動</span>
        </label>
      </div>

      <div class="flex gap-3">
        <button
          class="px-4 py-2 rounded-md bg-blue-600 text-white font-medium hover:bg-blue-700 disabled:opacity-50"
          onclick={save}
          disabled={saving || !scheduleValid}
        >{saving ? "保存中…" : "保存"}</button>
        <button
          class="px-4 py-2 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700 text-sm"
          onclick={() => api.openAppDir()}
        >設定フォルダを開く</button>
      </div>
    </div>
  {/if}

  {#if toast}
    <div class="rounded-md bg-slate-900 text-white px-4 py-2 text-sm">{toast}</div>
  {/if}
</section>
