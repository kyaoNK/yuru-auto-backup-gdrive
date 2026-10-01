<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "$lib/api";
  import { createAsyncScope } from "$lib/async-scope";
  import type { Config, DriveCandidate } from "$lib/types";

  const scope = createAsyncScope();
  let working = $state(false);
  let savedSnapshot = $state("");
  let config = $state<Config | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let toast = $state<string | null>(null);
  let detecting = $state(false);
  let driveCandidates = $state<DriveCandidate[]>([]);
  let showCandidates = $state(false);
  let newExcludedName = $state("");

  async function load() {
    const current = scope.ticket("config");
    loading = true;
    toast = null;
    try {
      const loaded = await api.getConfig();
      if (current()) { config = loaded; savedSnapshot = JSON.stringify(loaded); }
    } catch (e) { if (current()) toast = String(e); }
    finally { if (current()) loading = false; }
  }

  onMount(() => { void load(); return () => scope.dispose(); });

  async function pickSource() { await pick("source"); }
  async function pickDestination(startDir?: string) { await pick("destination", startDir); }
  async function pick(target: "source" | "destination" | "exclude", startDir?: string) {
    if (!config || working || saving) return;
    working = true;
    try {
      const initial = startDir ?? (target === "destination" ? config.destination : config.source);
      const picked = await api.pickFolder(initial ?? undefined);
      if (!scope.active || !config || !picked) return;
      if (target === "exclude") {
        if (!config.excludedFolders.some(p => p.toLowerCase() === picked.toLowerCase()))
          config.excludedFolders = [...config.excludedFolders, picked];
      } else {
        config[target] = picked;
        if (target === "destination") showCandidates = false;
      }
    } catch (e) { if (scope.active) toast = `フォルダ選択に失敗: ${e}`; }
    finally { if (scope.active) working = false; }
  }

  async function openAppDir() {
    try { await api.openAppDir(); }
    catch (e) { if (scope.active) toast = `設定フォルダを開けませんでした: ${e}`; }
  }

  async function detectDrives() {
    if (detecting) return;
    detecting = true;
    showCandidates = false;
    driveCandidates = [];
    try {
      const candidates = await api.detectDriveRoots();
      if (!scope.active) return;
      driveCandidates = candidates;
      if (driveCandidates.length === 0) {
        toast = "Google Drive が見つかりません。起動を確認してから手動で選んでください。";
      } else {
        showCandidates = true;
      }
    } catch (e) {
      if (scope.active) toast = String(e);
    } finally {
      if (scope.active) detecting = false;
    }
  }

  async function save() {
    if (!config || saving || working || detecting) return;
    saving = true;
    toast = null;
    try {
      const snapshot = JSON.parse(JSON.stringify(config)) as Config;
      await api.updateConfig(snapshot);
      if (scope.active) { savedSnapshot = JSON.stringify(config); toast = "保存しました"; }
    } catch (e) {
      if (scope.active) toast = String(e);
    } finally {
      if (scope.active) saving = false;
    }
  }

  const scheduleValid = $derived.by(() => {
    if (!config) return false;
    const m = /^(\d{2}):(\d{2})$/.exec(config.scheduleTime);
    if (!m) return false;
    const hour = Number(m[1]);
    const minute = Number(m[2]);
    return hour >= 0 && hour <= 23 && minute >= 0 && minute <= 59;
  });

  async function addExcludedFolder() { await pick("exclude"); }

  function removeExcludedFolder(p: string) {
    if (!config) return;
    config.excludedFolders = config.excludedFolders.filter((x) => x !== p);
  }

  function addExcludedName() {
    if (!config) return;
    const name = newExcludedName.trim();
    if (!name) return;
    if (/[\\/]/.test(name) || name === "." || name === "..") {
      toast = "除外名はパスではなくフォルダ名を指定してください。"; return;
    }
    if (!config.excludedFolderNames.some((n) => n.toLowerCase() === name.toLowerCase())) {
      config.excludedFolderNames = [...config.excludedFolderNames, name];
    }
    newExcludedName = "";
  }

  function removeExcludedName(n: string) {
    if (!config) return;
    config.excludedFolderNames = config.excludedFolderNames.filter((x) => x !== n);
  }
</script>

<section class="space-y-6">
  <h2 class="text-xl font-semibold">設定</h2>

  {#if loading}
    <p class="text-sm text-slate-500">読み込み中…</p>
  {:else if config}
    {#if savedSnapshot && JSON.stringify(config) !== savedSnapshot}
      <p role="status" class="text-sm text-amber-700 dark:text-amber-300">未保存の変更があります。画面を移動する前に保存してください。</p>
    {/if}
    <fieldset disabled={saving || working || detecting} class="space-y-5 min-w-0">
      <div class="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 p-4 space-y-2">
        <label class="block text-sm font-medium" for="source">監視元フォルダ</label>
        <div class="flex gap-2">
          <input
            id="source"
            type="text"
            readonly
            value={config.source ?? ""}
            placeholder="フォルダを選択してください"
            class="min-w-0 flex-1 rounded-md border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 px-3 py-2 text-sm"
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
            class="min-w-0 flex-1 rounded-md border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 px-3 py-2 text-sm"
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
        <p class="text-xs text-slate-500">検出は候補の推測です。選択先が実際に Google Drive と同期されていることを確認してください。</p>
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

      <div class="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 p-4 space-y-4">
        <div>
          <h3 class="text-sm font-medium">除外フォルダ（パス指定）</h3>
          <p class="text-xs text-slate-500 mt-1">
            選んだフォルダ配下はバックアップの対象から外れます（サブツリー全体）。
          </p>
        </div>
        {#if config.excludedFolders.length > 0}
          <ul class="divide-y divide-slate-200 dark:divide-slate-700 border border-slate-200 dark:border-slate-700 rounded-md">
            {#each config.excludedFolders as p}
              <li class="px-3 py-2 flex items-center justify-between gap-2">
                <span class="text-xs font-mono break-all min-w-0">{p}</span>
                <button
                  class="text-xs px-2 py-1 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-red-50 dark:hover:bg-red-900/30 hover:text-red-600 shrink-0"
                  onclick={() => removeExcludedFolder(p)}
                >削除</button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="text-xs text-slate-500">除外フォルダはありません。</p>
        {/if}
        <button
          class="text-sm px-3 py-2 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700"
          onclick={addExcludedFolder}
          disabled={!config.source}
        >＋ 除外フォルダを追加</button>
        {#if !config.source}
          <p class="text-xs text-slate-500">監視元フォルダを先に設定してください。</p>
        {/if}

        <hr class="border-slate-200 dark:border-slate-700" />

        <div>
          <h3 class="text-sm font-medium">除外フォルダ名（パターン）</h3>
          <p class="text-xs text-slate-500 mt-1">
            この名前のフォルダ配下はすべて除外されます（大文字小文字を区別しません）。例: <code>Cache</code>, <code>Proxy</code>
          </p>
        </div>
        {#if config.excludedFolderNames.length > 0}
          <ul class="flex flex-wrap gap-2">
            {#each config.excludedFolderNames as n}
              <li class="inline-flex items-center gap-1 rounded-full bg-slate-100 dark:bg-slate-700 px-3 py-1 text-xs">
                <span class="font-mono">{n}</span>
                <button
                  class="text-slate-500 hover:text-red-600"
                  aria-label={`除外名 ${n} を削除`}
                  onclick={() => removeExcludedName(n)}
                >×</button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="text-xs text-slate-500">除外フォルダ名はありません。</p>
        {/if}
        <div class="flex gap-2">
          <input
            type="text"
            placeholder="例: Cache"
            bind:value={newExcludedName}
            aria-label="追加する除外フォルダ名"
            onkeydown={(e) => !e.isComposing && e.key === "Enter" && (e.preventDefault(), addExcludedName())}
            class="min-w-0 flex-1 rounded-md border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 px-3 py-2 text-sm"
          />
          <button
            class="text-sm px-3 py-2 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700"
            onclick={addExcludedName}
          >追加</button>
        </div>
      </div>

      <div class="flex gap-3">
        <button
          class="px-4 py-2 rounded-md bg-blue-600 text-white font-medium hover:bg-blue-700 disabled:opacity-50"
          onclick={save}
          disabled={saving || !scheduleValid}
        >{saving ? "保存中…" : "保存"}</button>
        <button
          class="px-4 py-2 rounded-md border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700 text-sm"
          onclick={openAppDir}
        >設定フォルダを開く</button>
      </div>
    </fieldset>
  {:else}
    <p class="text-sm">設定を読み込めませんでした。設定ファイルを確認して再読み込みしてください。</p>
    <button class="underline" onclick={load}>再読み込み</button>
    <button class="ml-3 underline" onclick={openAppDir}>設定フォルダを開く</button>
  {/if}

  {#if toast}
    <div role="status" class="break-words rounded-md bg-slate-900 text-white px-4 py-2 text-sm">{toast}</div>
  {/if}
</section>
