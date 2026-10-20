<script lang="ts">
  import Icon from './Icon.svelte'
  import type { IconName } from '../lib/icons'
  import { app, settings, activeTab, navigate, newTab, activate, toggleSplit, newFolder, HOME, SYNC, SETTINGS } from '../lib/state.svelte'
  import { api } from '../lib/api'
  import { basename, fuzzy, samePath } from '../lib/util'

  type Item = { label: string; sub?: string; icon: IconName; run: () => void; score?: number }

  let q = $state('')
  let index = $state(0)
  let input = $state<HTMLInputElement>()
  let sub = $state<Item[]>([])
  $effect(() => {
    input?.focus()
  })

  const t = activeTab()
  // Podfoldery bieżącego folderu też są w wynikach.
  if (!t.special) {
    api.listDir(t.path).then((l) => {
      sub = l.filter((e) => e.dir && !e.hidden).slice(0, 300).map((e) => ({ label: e.name, sub: e.path, icon: 'folder', run: () => navigate(t, e.path) }))
    }).catch(() => {})
  }

  const base = $derived.by(() => {
    const out: Item[] = []
    const seen = new Set<string>()
    const add = (it: Item, key?: string) => {
      const k = (key ?? it.sub ?? it.label).toLowerCase()
      if (seen.has(k)) return
      seen.add(k)
      out.push(it)
    }
    for (const tb of app.tabs) add({ label: tb.title, sub: 'Karta · ' + (tb.special ? '' : tb.path), icon: 'copy', run: () => activate(tb.id) }, 'tab:' + tb.id)
    for (const f of app.favorites) add({ label: f.name, sub: f.path, icon: 'pin', run: () => navigate(t, f.path) })
    for (const k of app.known) add({ label: k.name, sub: k.path, icon: 'folder', run: () => navigate(t, k.path) })
    for (const d of app.drives) add({ label: `${d.label || 'Dysk'} (${d.letter})`, sub: d.remote ?? d.path, icon: d.kind === 'network' ? 'network' : 'drive', run: () => navigate(t, d.path) }, d.path)
    for (const r of app.recent) add({ label: basename(r), sub: r, icon: 'history', run: () => navigate(t, r) })
    for (const s of sub) add(s)
    const cmd = (label: string, icon: IconName, run: () => void) => add({ label, sub: 'Polecenie', icon, run }, 'cmd:' + label)
    cmd('Ten komputer', 'pc', () => navigate(t, HOME))
    cmd('Synchronizacja', 'sync', () => navigate(t, SYNC))
    cmd('Ustawienia', 'settings', () => navigate(t, SETTINGS))
    cmd('Nowa karta', 'plus', () => newTab(t.path))
    cmd('Nowy folder', 'folderPlus', () => newFolder(t))
    cmd('Podział na dwa panele', 'split', toggleSplit)
    cmd('Podłącz dysk sieciowy', 'network', () => (app.dialog = { type: 'network' }))
    cmd('Nowa synchronizacja', 'sync', () => (app.dialog = { type: 'sync', profile: null, source: t.special ? undefined : t.path }))
    cmd(settings.showHidden ? 'Ukryj ukryte pliki' : 'Pokaż ukryte pliki', 'eye', () => (settings.showHidden = !settings.showHidden))
    if (!t.special) cmd('Otwórz terminal tutaj', 'terminal', () => api.terminal(t.path))
    return out
  })

  const results = $derived.by(() => {
    const query = q.trim()
    const list: Item[] = []
    if (/^([a-zA-Z]:|\\\\)/.test(query)) list.push({ label: `Przejdź do ${query}`, icon: 'fwd', run: () => navigate(t, query), score: 1e9 })
    for (const it of base) {
      const s = Math.max(fuzzy(query, it.label), fuzzy(query, it.sub ?? '') * 0.6)
      if (s >= 0) list.push({ ...it, score: s })
    }
    if (query) list.sort((a, b) => (b.score ?? 0) - (a.score ?? 0))
    return list.slice(0, 60)
  })
  $effect(() => {
    q
    index = 0
  })

  function run(it: Item) {
    app.palette = false
    it.run()
  }

  function onKey(ev: KeyboardEvent) {
    if (ev.key === 'Escape') app.palette = false
    else if (ev.key === 'ArrowDown') index = Math.min(results.length - 1, index + 1)
    else if (ev.key === 'ArrowUp') index = Math.max(0, index - 1)
    else if (ev.key === 'Enter' && results[index]) {
      const it = results[index]
      if (ev.ctrlKey && it.sub && !samePath(it.sub, '') && it.icon !== 'copy') {
        app.palette = false
        newTab(it.sub)
      } else run(it)
    } else return
    ev.preventDefault()
    ev.stopPropagation()
    document.querySelector('.pal .on')?.scrollIntoView({ block: 'nearest' })
  }
</script>

<div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && (app.palette = false)}>
  <div class="pal fade-in">
    <div class="in">
      <Icon name="search" />
      <input bind:this={input} bind:value={q} placeholder="Przejdź do folderu, dysku, karty lub polecenia…" spellcheck="false" onkeydown={onKey} />
      <span class="kbd">Ctrl+Enter – nowa karta</span>
    </div>
    <div class="list">
      {#each results as it, i (it.label + (it.sub ?? '') + i)}
        <button class:on={i === index} onmouseenter={() => (index = i)} onclick={() => run(it)}>
          <Icon name={it.icon} size={16} />
          <span class="ellipsis lbl">{it.label}</span>
          {#if it.sub}<span class="ellipsis faint sub">{it.sub}</span>{/if}
        </button>
      {:else}
        <p class="faint">Brak wyników</p>
      {/each}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 350;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    justify-content: center;
    padding-top: 12vh;
  }
  .pal {
    width: 640px;
    max-width: calc(100% - 32px);
    max-height: 64vh;
    display: flex;
    flex-direction: column;
    background: var(--high);
    border: 1px solid var(--line);
    border-radius: 18px;
    box-shadow: 0 40px 100px rgba(0, 0, 0, 0.8);
    overflow: hidden;
  }
  .in {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 16px;
    color: var(--faint);
  }
  .in input {
    flex: 1;
    border: 0;
    background: transparent;
    font-size: 15px;
    padding: 12px 0;
  }
  .list {
    overflow-y: auto;
    padding: 6px;
  }
  button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 12px;
    border-radius: 10px;
    color: var(--dim);
    text-align: left;
  }
  button.on {
    background: var(--press);
    color: var(--text);
  }
  .lbl {
    flex: none;
    max-width: 45%;
  }
  .sub {
    flex: 1;
    font-size: 12px;
    text-align: right;
  }
  p {
    padding: 12px;
    margin: 0;
  }
</style>
