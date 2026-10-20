<script lang="ts">
  import Icon from './Icon.svelte'
  import type { IconName } from '../lib/icons'
  import {
    app, settings, activeTab, navigate, newTab, activate, closeTab, reopenClosed, togglePin, toggleSplit, newFolder, newFile, paste, copySel, trash,
    back, forward, up, load, savePrefs, setSort, setGroup, startSearch, saveSearch, runSaved, undo, redo, addFavorite, toggleHomeDir, isHomeDir,
    extractHere, openEntry, refreshSync, openSpace, shelfAdd, toast, SORT_LABEL, GROUP_LABEL, HOME, SYNC, SETTINGS, type SortKey, type GroupKey,
  } from '../lib/state.svelte'
  import { api, type Entry } from '../lib/api'
  import { basename, fuzzy, kindOf, isBrowsableArchive, type Kind } from '../lib/util'

  type Group = 'Ostatnio' | 'Polecenia' | 'Widok' | 'Ustawienia' | 'Karty' | 'Miejsca' | 'Pliki' | 'Wyszukiwania'
  type Item = { id: string; label: string; sub?: string; icon: IconName; group: Group; kbd?: string; path?: string; run: () => void; score?: number }

  let q = $state(app.paletteMode === 'cmd' ? '>' : '')
  let index = $state(0)
  let input = $state<HTMLInputElement>()
  let listEl = $state<HTMLDivElement>()
  let sub = $state<Entry[]>([])
  $effect(() => {
    input?.focus()
  })

  const t = activeTab()
  // Zawartość bieżącego folderu (foldery i pliki) też jest w wynikach.
  if (!t.special) sub = t.entries.length ? t.entries : []
  if (!t.special && !t.entries.length) api.listDir(t.path).then((l) => (sub = l)).catch(() => {})

  // Ostatnio użyte polecenia – na górze pustej palety.
  const RECENT_KEY = 'ofp:palette-recent'
  let recentIds: string[] = []
  try {
    recentIds = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]')
  } catch {
    recentIds = []
  }

  const KIND_ICON: Partial<Record<Kind, IconName>> = { image: 'image', raw: 'image', video: 'video', audio: 'music', pdf: 'doc', doc: 'doc', sheet: 'sheet', slides: 'doc', text: 'doc', code: 'code', archive: 'archive', app: 'app', folder: 'folder' }

  const sel = t.selectedEntries
  const one = sel.length === 1 ? sel[0] : null

  const items = $derived.by(() => {
    const out: Item[] = []
    const seen = new Set<string>()
    const add = (it: Item) => {
      if (seen.has(it.id)) return
      seen.add(it.id)
      out.push(it)
    }
    const cmd = (id: string, label: string, icon: IconName, run: () => void, kbd?: string, group: Group = 'Polecenia') => add({ id: 'cmd:' + id, label, icon, run, kbd, group })

    // ---------- Polecenia ----------
    cmd('home', 'Przejdź do: Komputer', 'pc', () => navigate(t, HOME))
    cmd('sync', 'Przejdź do: Synchronizacja', 'sync', () => navigate(t, SYNC))
    cmd('settings', 'Otwórz ustawienia', 'settings', () => navigate(t, SETTINGS))
    if (!t.special) {
      cmd('newfolder', 'Nowy folder', 'folderPlus', () => newFolder(t), 'Ctrl+Shift+N')
      cmd('newfile', 'Nowy plik tekstowy', 'filePlus', () => newFile(t))
      cmd('paste', 'Wklej', 'paste', () => paste(t), 'Ctrl+V')
      cmd('selall', 'Zaznacz wszystko', 'check', () => t.items.forEach((e) => t.selected.add(e.path)), 'Ctrl+A')
      cmd('refresh', 'Odśwież', 'refresh', () => load(t, true), 'F5')
      cmd('up', 'W górę', 'up', () => up(t), 'Alt+↑')
      cmd('copypath', 'Kopiuj ścieżkę folderu', 'link', () => { navigator.clipboard.writeText(t.path); toast('Skopiowano ścieżkę') })
      cmd('terminal', 'Otwórz terminal tutaj', 'terminal', () => api.terminal(t.path))
      cmd('explorer', 'Otwórz w Eksploratorze', 'open', () => api.open(t.path))
      cmd('fav', 'Przypnij ten folder w panelu bocznym', 'pin', () => addFavorite(t.path))
      cmd('homedir', isHomeDir(t.path) ? 'Usuń ten folder z ekranu Komputer' : 'Pokaż ten folder na ekranie Komputer', 'grid', () => toggleHomeDir(t.path))
      cmd('space', 'Zajętość i porządki w tym folderze', 'grid', () => openSpace(t, t.path))
      cmd('shelf', 'Dodaj zaznaczone do schowka plików', 'archive', () => shelfAdd(t.selectedEntries), 'Ctrl+Shift+S')
      if (t.search) cmd('savesearch', 'Zapisz bieżące wyszukiwanie', 'star', () => saveSearch(t))
    }
    if (t.hIndex > 0) cmd('back', 'Wstecz', 'back', () => back(t), 'Alt+←')
    if (t.hIndex < t.history.length - 1) cmd('fwd', 'Dalej', 'fwd', () => forward(t), 'Alt+→')
    if (sel.length) {
      cmd('copy', `Kopiuj zaznaczone (${sel.length})`, 'copy', () => copySel(t, false), 'Ctrl+C')
      cmd('cut', `Wytnij zaznaczone (${sel.length})`, 'cut', () => copySel(t, true), 'Ctrl+X')
      cmd('trash', `Przenieś do Kosza (${sel.length})`, 'trash', () => trash(t), 'Del')
      cmd('props', 'Właściwości zaznaczonych', 'info', () => (app.dialog = { type: 'props', entries: sel }), 'Alt+Enter')
      if (sel.length > 1) cmd('bulk', `Zmień nazwy (${sel.length})…`, 'edit', () => (app.dialog = { type: 'bulkRename', entries: sel }), 'F2')
    }
    if (one) {
      cmd('rename', `Zmień nazwę „${one.name}”`, 'edit', () => (t.renaming = one.path), 'F2')
      if (isBrowsableArchive(one)) cmd('extract', `Wypakuj „${one.name}” tutaj`, 'download', () => extractHere(one.path))
    }
    cmd('undo', 'Cofnij ostatnią operację', 'undo', undo, 'Ctrl+Z')
    cmd('redo', 'Ponów cofniętą operację', 'undo', redo, 'Ctrl+Y')
    cmd('network', 'Połącz z lokalizacją sieciową…', 'network', () => (app.dialog = { type: 'network' }))
    cmd('syncnow', 'Synchronizuj wszystko teraz', 'sync', () => api.syncNow().then(refreshSync))
    cmd('update', 'Sprawdź aktualizacje', 'download', () => api.checkUpdate().then((u) => toast(u.newer ? `Dostępna wersja ${u.latest}` : 'Masz najnowszą wersję', u.newer ? 'info' : 'ok')))

    // ---------- Karty ----------
    cmd('newtab', 'Nowa karta', 'plus', () => newTab(t.path, true, t.id), 'Ctrl+T', 'Karty')
    cmd('dup', 'Duplikuj kartę', 'copy', () => newTab(t.path, true, t.id), undefined, 'Karty')
    cmd('pin', t.pinned ? 'Odepnij kartę' : 'Przypnij kartę', 'pin', () => togglePin(t), undefined, 'Karty')
    if (!t.pinned) cmd('close', 'Zamknij kartę', 'close', () => closeTab(t.id), 'Ctrl+W', 'Karty')
    if (app.closed.length) cmd('reopen', 'Przywróć zamkniętą kartę', 'history', reopenClosed, 'Ctrl+Shift+T', 'Karty')
    cmd('split', app.split ? 'Wyłącz podział na dwa panele' : 'Podział na dwa panele', 'split', toggleSplit, 'Ctrl+\\', 'Karty')
    for (const tb of app.tabs) if (tb.id !== t.id) add({ id: 'tab:' + tb.id, label: tb.title, sub: tb.special ? '' : tb.path, icon: tb.pinned ? 'pin' : 'copy', group: 'Karty', run: () => activate(tb.id) })

    // ---------- Widok ----------
    if (!t.special) {
      const view = (v: 'list' | 'grid' | 'gallery', label: string, icon: IconName, kbd: string) => cmd('view:' + v, `Widok: ${label}`, icon, () => { t.view = v; savePrefs(t) }, kbd, 'Widok')
      view('list', 'lista', 'list', 'Ctrl+1')
      view('grid', 'siatka', 'grid', 'Ctrl+2')
      view('gallery', 'galeria', 'gallery', 'Ctrl+3')
      for (const k of Object.keys(SORT_LABEL) as SortKey[]) cmd('sort:' + k, `Sortuj według: ${SORT_LABEL[k].toLowerCase()}`, 'sort', () => setSort(t, k), undefined, 'Widok')
      for (const g of Object.keys(GROUP_LABEL) as GroupKey[]) cmd('group:' + g, g === 'none' ? 'Bez grupowania' : `Grupuj według: ${GROUP_LABEL[g].toLowerCase()}`, 'list', () => setGroup(t, g), undefined, 'Widok')
    }
    cmd('shelfToggle', settings.shelfOpen ? 'Ukryj schowek plików' : 'Pokaż schowek plików', 'archive', () => (settings.shelfOpen = !settings.shelfOpen), undefined, 'Widok')
    cmd('terminal', settings.terminalOpen ? 'Ukryj terminal' : 'Pokaż terminal', 'terminal', () => (settings.terminalOpen = !settings.terminalOpen), 'Ctrl+`', 'Widok')
    cmd('preview', settings.previewPane ? 'Ukryj panel podglądu' : 'Pokaż panel podglądu', 'panel', () => (settings.previewPane = !settings.previewPane), 'Ctrl+P', 'Widok')
    cmd('sidebar', app.sidebarOpen ? 'Ukryj panel boczny' : 'Pokaż panel boczny', 'sidebar', () => (app.sidebarOpen = !app.sidebarOpen), 'Ctrl+B', 'Widok')
    cmd('hidden', settings.showHidden ? 'Ukryj ukryte pliki' : 'Pokaż ukryte pliki', 'eye', () => (settings.showHidden = !settings.showHidden), 'Ctrl+H', 'Widok')

    // ---------- Ustawienia ----------
    const theme = (v: 'system' | 'dark' | 'light', label: string) => cmd('theme:' + v, `Motyw: ${label}`, 'eye', () => (settings.theme = v), undefined, 'Ustawienia')
    theme('dark', 'ciemny')
    theme('light', 'jasny')
    theme('system', 'jak w systemie')
    const flag = (k: keyof typeof settings, label: string) =>
      cmd('set:' + k, `${settings[k] ? 'Wyłącz' : 'Włącz'}: ${label}`, 'settings', () => ((settings as Record<string, unknown>)[k] = !settings[k]), undefined, 'Ustawienia')
    flag('foldersFirst', 'foldery przed plikami')
    flag('singleClickOpen', 'otwieranie jednym kliknięciem')
    flag('autoGallery', 'galerię dla folderów ze zdjęciami')
    flag('folderSizes', 'rozmiary folderów')
    flag('showQuickAccess', 'szybki dostęp w panelu bocznym')
    flag('showRecent', 'ostatnio otwierane na ekranie Komputer')
    flag('confirmTrash', 'pytanie przed przeniesieniem do Kosza')
    flag('playVideoPreview', 'odtwarzanie wideo w podglądzie')

    // ---------- Wyszukiwania ----------
    for (const s of settings.savedSearches) add({ id: 'saved:' + s.id, label: s.name, sub: `${s.query} · ${s.root}`, icon: 'search', group: 'Wyszukiwania', run: () => runSaved(t, s) })

    // ---------- Miejsca ----------
    const place = (label: string, path: string, icon: IconName) => add({ id: 'p:' + path.toLowerCase(), label, sub: path, path, icon, group: 'Miejsca', run: () => navigate(t, path) })
    for (const f of app.favorites) place(f.name, f.path, 'pin')
    for (const k of app.known) place(k.name, k.path, 'folder')
    for (const d of app.drives) place(`${d.label || 'Dysk'} (${d.letter})`, d.path, d.kind === 'network' ? 'network' : 'drive')
    for (const p of settings.netPlaces) place(p.name, p.path, 'network')
    for (const r of app.recent) place(basename(r) || r, r, 'history')

    // ---------- Pliki w bieżącym folderze ----------
    for (const e of sub.slice(0, 2000)) {
      if (e.hidden && !settings.showHidden) continue
      add({ id: 'f:' + e.path.toLowerCase(), label: e.name, sub: e.dir ? 'Folder' : basename(t.path), path: e.dir ? e.path : undefined, icon: KIND_ICON[kindOf(e)] ?? 'file', group: e.dir ? 'Miejsca' : 'Pliki', run: () => openEntry(t, e) })
    }
    return out
  })

  // Prefiksy: „>” – tylko polecenia, „@” – pliki w folderze, „?” – szukaj w podfolderach.
  const mode = $derived(q.startsWith('>') ? 'cmd' : q.startsWith('@') ? 'file' : q.startsWith('?') ? 'search' : 'all')
  const query = $derived((mode === 'all' ? q : q.slice(1)).trim())

  const results = $derived.by(() => {
    if (mode === 'search') {
      return query && !t.special
        ? [{ id: 'search', label: `Szukaj „${query}” w ${t.title} i podfolderach`, icon: 'search' as IconName, group: 'Polecenia' as Group, kbd: 'Enter', run: () => startSearch(t, query), score: 1 }]
        : []
    }
    const list: Item[] = []
    if (mode === 'all' && /^([a-zA-Z]:|\\\\|ftp:\\\\)/i.test(query)) list.push({ id: 'go', label: `Przejdź do ${query}`, icon: 'fwd', group: 'Miejsca', run: () => navigate(t, query), score: 1e9 })
    const pool = items.filter((it) => (mode === 'cmd' ? it.id.startsWith('cmd:') : mode === 'file' ? it.id.startsWith('f:') : true))
    if (!query) {
      // Pusta paleta: ostatnio użyte, potem reszta w naturalnej kolejności.
      const rec = recentIds.map((id) => pool.find((x) => x.id === id)).filter((x): x is Item => !!x)
      return [...rec.map((x) => ({ ...x, group: 'Ostatnio' as Group })), ...pool.filter((x) => !recentIds.includes(x.id))].slice(0, 80)
    }
    for (const it of pool) {
      const s = Math.max(fuzzy(query, it.label), fuzzy(query, it.sub ?? '') * 0.5)
      // Polecenia lekko wyżej niż pliki o podobnym dopasowaniu.
      if (s >= 0) list.push({ ...it, score: s + (it.id.startsWith('cmd:') ? 2 : 0) + (recentIds.includes(it.id) ? 5 : 0) })
    }
    list.sort((a, b) => (b.score ?? 0) - (a.score ?? 0))
    return list.slice(0, 80)
  })
  $effect(() => {
    q
    index = 0
  })

  function remember(id: string) {
    if (!id.startsWith('cmd:')) return
    recentIds = [id, ...recentIds.filter((x) => x !== id)].slice(0, 6)
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(recentIds))
    } catch {
      /* brak dostępu do pamięci – trudno */
    }
  }

  function run(it: Item, newTabToo = false) {
    app.palette = false
    remember(it.id)
    if (newTabToo && it.path) newTab(it.path)
    else it.run()
  }

  function onKey(ev: KeyboardEvent) {
    if (ev.key === 'Escape') app.palette = false
    else if (ev.key === 'ArrowDown') index = Math.min(results.length - 1, index + 1)
    else if (ev.key === 'ArrowUp') index = Math.max(0, index - 1)
    else if (ev.key === 'PageDown') index = Math.min(results.length - 1, index + 8)
    else if (ev.key === 'PageUp') index = Math.max(0, index - 8)
    else if (ev.key === 'Enter' && results[index]) run(results[index], ev.ctrlKey)
    else return
    ev.preventDefault()
    ev.stopPropagation()
    requestAnimationFrame(() => listEl?.querySelector('.on')?.scrollIntoView({ block: 'nearest' }))
  }

  const PLACEHOLDER: Record<string, string> = {
    all: 'Folder, plik, karta lub polecenie…',
    cmd: 'Polecenie…',
    file: 'Plik w tym folderze…',
    search: 'Czego szukać w podfolderach?',
  }
</script>

<div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && (app.palette = false)}>
  <div class="pal fade-in" role="dialog" aria-label="Paleta poleceń" aria-modal="true">
    <div class="in">
      <Icon name={mode === 'cmd' ? 'bolt' : mode === 'search' ? 'search' : mode === 'file' ? 'file' : 'search'} />
      <input
        bind:this={input}
        bind:value={q}
        placeholder={PLACEHOLDER[mode]}
        spellcheck="false"
        onkeydown={onKey}
        role="combobox"
        aria-expanded="true"
        aria-controls="pal-list"
        aria-activedescendant={results[index] ? `pal-${index}` : undefined}
      />
    </div>
    <div class="modes">
      <button class:on={mode === 'all'} onclick={() => { q = query; input?.focus() }}>Wszystko</button>
      <button class:on={mode === 'cmd'} onclick={() => { q = '>' + query; input?.focus() }}><span class="kbd">&gt;</span> Polecenia</button>
      <button class:on={mode === 'file'} onclick={() => { q = '@' + query; input?.focus() }}><span class="kbd">@</span> Pliki</button>
      <button class:on={mode === 'search'} onclick={() => { q = '?' + query; input?.focus() }}><span class="kbd">?</span> Szukaj głębiej</button>
      <span class="grow"></span>
      <span class="kbd hint">↑↓ wybór · Enter · Ctrl+Enter – nowa karta</span>
    </div>
    <div class="list" id="pal-list" role="listbox" bind:this={listEl}>
      {#each results as it, i (it.id + i)}
        {#if i === 0 || results[i - 1].group !== it.group}<div class="gh" role="presentation">{it.group}</div>{/if}
        <button id="pal-{i}" class:on={i === index} role="option" aria-selected={i === index} onmouseenter={() => (index = i)} onclick={(e) => run(it, e.ctrlKey)}>
          <Icon name={it.icon} size={16} />
          <span class="ellipsis lbl">{it.label}</span>
          {#if it.sub}<span class="ellipsis faint sub">{it.sub}</span>{:else}<span class="grow"></span>{/if}
          {#if it.kbd}<span class="kbd">{it.kbd}</span>{/if}
        </button>
      {:else}
        <p class="faint">{mode === 'search' ? 'Wpisz, czego szukać' : 'Brak wyników'}</p>
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
    padding-top: 10vh;
  }
  .pal {
    width: 680px;
    max-width: calc(100% - 32px);
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    background: var(--popup-bg);
    backdrop-filter: var(--popup-blur);
    border: 1px solid var(--line);
    border-radius: var(--r-xl);
    box-shadow: 0 40px 100px var(--shadow);
    overflow: hidden;
    align-self: flex-start;
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
  .modes {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 10px 8px;
    border-bottom: 1px solid var(--line);
  }
  .modes button {
    width: auto;
    padding: 4px 10px;
    border-radius: var(--r-s);
    font-size: 12px;
    color: var(--faint);
  }
  .modes button:hover {
    color: var(--text);
    background: var(--hover);
  }
  .modes button.on {
    background: var(--accent);
    color: var(--on-accent);
  }
  .modes button.on .kbd {
    color: inherit;
  }
  .hint {
    font-size: 10.5px;
  }
  .list {
    overflow-y: auto;
    padding: 6px;
  }
  .gh {
    padding: 8px 12px 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--ghost);
  }
  .list button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    border-radius: var(--r);
    color: var(--dim);
    text-align: left;
  }
  .list button.on {
    background: var(--press);
    color: var(--text);
  }
  .lbl {
    flex: none;
    max-width: 55%;
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
