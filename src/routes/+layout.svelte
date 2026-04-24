<script lang="ts">
  import "../app.css";
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
    <div class="max-w-3xl mx-auto px-6 py-3 flex items-center gap-6">
      <h1 class="font-semibold">yuru-auto-backup-gdrive</h1>
      <nav class="flex gap-1 text-sm">
        {#each navItems as item}
          <a
            href={item.href}
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
    {@render children()}
  </main>
</div>
