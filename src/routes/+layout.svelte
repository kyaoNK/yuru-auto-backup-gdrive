<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { listen } from "@tauri-apps/api/event";
  import { createAsyncScope } from "$lib/async-scope";
  const scope = createAsyncScope();
  let navigationError = $state<string | null>(null);
  onMount(() => {
    void scope.own(listen("navigate-settings", () => {
      if (scope.active) void goto("/settings").catch(e => { navigationError = String(e); });
    }), e => { navigationError = String(e); });
    return () => scope.dispose();
  });
  import { page } from "$app/state";
  let { children } = $props();

  const navItems = [
    { href: "/", label: "ダッシュボード" },
    { href: "/settings", label: "設定" },
    { href: "/logs", label: "ログ" },
  ];

  const isActive = (href: string) =>
    href === "/"
      ? page.url.pathname === "/"
      : page.url.pathname.startsWith(href);
</script>

<div class="min-h-screen flex flex-col bg-slate-50 text-slate-900 dark:bg-slate-900 dark:text-slate-100">
  <header class="border-b border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-800">
    <div class="max-w-3xl mx-auto px-6 py-3 flex flex-wrap items-center gap-3">
      <h1 class="font-semibold">yuru-auto-backup-gdrive</h1>
      <nav class="flex gap-1 text-sm">
        {#each navItems as item}
          <a
            href={item.href}
            aria-current={isActive(item.href) ? "page" : undefined}
            class="px-3 py-1.5 rounded-md transition-colors"
            class:bg-slate-200={isActive(item.href)}
            class:dark:bg-slate-700={isActive(item.href)}
            class:hover:bg-slate-100={!isActive(item.href)}
            class:dark:hover:bg-slate-700={!isActive(item.href)}
          >
            {item.label}
          </a>
        {/each}
      </nav>
    </div>
  </header>
  <main class="flex-1 max-w-3xl w-full mx-auto px-6 py-6">
    {#if navigationError}<p role="alert" class="text-sm text-red-600">{navigationError}</p>{/if}
    {@render children()}
  </main>
</div>
