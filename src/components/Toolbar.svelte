<script lang="ts">
  import type { IconName } from '../lib/icons'
  import Icon from './Icon.svelte'
  import {
    app, settings, newFolderWith, zipSelection, toast, back, forward, up, load, navigate, startSearch, closeSearch, saveSearch, transfer, dropMode, newFolder, newFile,
    copySel, paste, trash, savePrefs, toggleSplit, selectOnly, sortMenuItems, SORT_LABEL, GROUP_LABEL, type Tab, type SortKey, type KindFilter,
  } from '../lib/state.svelte'
  import { api, thumbUrl, type Entry } from '../lib/api'
  import { crumbs, collator, parent, samePath } from '../lib/util'
  import { QUERY_HINTS, parseQuery } from '../lib/query'

  let { tab, focused }: { tab: Tab; focused: boolean } = $props()

  let editing = $state(false)
  let addr = $state('')
  let sugg = $state<Entry[]>([])
  let suggIndex = $state(-1)
  let addrInput = $state<HTMLInputElement>()
  let searchInput = $state<HTMLInputElement>()
  let q = $state('')
  let over = $state<string | null>(null)

  const parts = $derived(crumbs(tab.path))
  const sel = $derived(tab.selectedEntries)

  export function focusAddress() {
    startEdit()
  }
  export function focusSearch() {
    searchInput?.focus()
    searchInput?.select()
  }

  function startEdit() {
    if (tab.special) return
    editing = true
    addr = tab.path
    sugg = []
    queueMicrotask(() => {
      addrInput?.focus()
      addrInput?.select()
    })
  }

  let suggToken = 0
  async function suggest() {
    const v = addr.replace(/\//g, '\\')
    const i = v.lastIndexOf('\\')
    if (i < 0) return (sugg = [])
    const dir = v.slice(0, i + 1)
    const pre = v.slice(i + 1).toLowerCase()
    const token = ++suggToken
    try {
      const list = await api.listDir(dir)
      if (token !== suggToken) return
      sugg = list
        .filter((e) => e.dir && (!pre || e.name.toLowerCase().startsWith(pre)) && (settings.showHidden || !e.hidden))
        .sort((a, b) => collator.compare(a.name, b.name))
        .slice(0, 12)
      suggIndex = -1
    } catch {
      sugg = []
    }
  }

  async function commit(path: string) {
    editing = false
    sugg = []
    path = path.trim().replace(/\//g, '\\').replace(/^"|"$/g, '')
    if (!path || samePath(path, tab.path)) return
    try {
      const st = await api.stat(path)
      if (st.dir) navigate(tab, st.path)
      else {
        // Wklejona ścieżka do pliku: otwieramy folder i zaznaczamy plik.
        const p = parent(st.path)
        if (p) {
          navigate(tab, p)
          setTimeout(() => selectOnly(tab, st.path), 250)
        }
      }
    } catch (e) {
      const { toast } = await import('../lib/state.svelte')
      toast(`Nie można otworzyć „${path}”: ${e}`, 'error')
    }
  }

  function addrKey(e: KeyboardEvent) {
    if (e.key === 'Enter') commit(suggIndex >= 0 ? sugg[suggIndex].path : addr)
    else if (e.key === 'Escape') {
      editing = false
      sugg = []
    } else if (e.key === 'ArrowDown' && sugg.length) {
      e.preventDefault()
      suggIndex = (suggIndex + 1) % sugg.length
    } else if (e.key === 'ArrowUp' && sugg.length) {
      e.preventDefault()
      suggIndex = (suggIndex - 1 + sugg.length) % sugg.length
    } else if (e.key === 'Tab' && sugg.length) {
      e.preventDefault()
      addr = (suggIndex >= 0 ? sugg[suggIndex] : sugg[0]).path + '\\'
      suggest()
    }
  }

  async function crumbMenu(e: MouseEvent, path: string) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    const list = (await api.listDir(path).catch(() => [] as Entry[]))
      .filter((x) => x.dir && (settings.showHidden || !x.hidden))
      .sort((a, b) => collator.compare(a.name, b.name))
    app.menu = {
      x: r.left,
      y: r.bottom + 4,
      items: list.length
        ? list.slice(0, 60).map((x) => ({ label: x.name, icon: 'folder' as const, run: () => navigate(tab, x.path) }))
        : [{ label: 'Brak podfolderów', disabled: true, run: () => {} }],
    }
  }

  let searchFocus = $state(false)
  const liveChips = $derived(parseQuery(q).chips)
  const savedHere = $derived(!!tab.search && settings.savedSearches.some((s) => s.query === tab.search!.query && samePath(s.root, tab.path)))
  function addHint(h: string) {
    q = (q.trim() ? q.trim() + ' ' : '') + h + ' '
    searchInput?.focus()
  }

  function onSearchKey(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      startSearch(tab, q)
    } else if (e.key === 'Escape') {
      q = ''
      tab.filter = ''
      if (tab.search) closeSearch(tab)
      ;(e.target as HTMLInputElement).blur()
    }
  }
  $effect(() => {
    // Wpisywanie filtruje bieżący folder na żywo; Enter szuka w podfolderach.
    if (!tab.search) tab.filter = q
  })
  $effect(() => {
    tab.path
    q = ''
  })


  function sortMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    app.menu = {
      x: r.left, y: r.bottom + 4,
      items: [
        ...sortMenuItems(tab),
        { sep: true },
        { label: (settings.foldersFirst ? '✓ ' : '   ') + 'Foldery na początku', run: () => (settings.foldersFirst = !settings.foldersFirst) },
        { label: (settings.showHidden ? '✓ ' : '   ') + 'Ukryte pliki', kbd: 'Ctrl+H', run: () => (settings.showHidden = !settings.showHidden) },
      ],
    }
  }

  async function newFolderNamed(name: string) {
    try {
      const p = await api.mkdir(tab.path, name)
      await load(tab)
      selectOnly(tab, p)
    } catch (err) {
      toast(String(err), 'error')
    }
  }

  function newMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    app.menu = {
      x: r.left, y: r.bottom + 4,
      items: [
        { label: 'Folder', icon: 'folderPlus', kbd: 'Ctrl+Shift+N', run: () => newFolder(tab) },
        { label: 'Folder z dzisiejszą datą', icon: 'folderPlus', run: () => newFolderNamed(new Date().toLocaleDateString('sv')) },
        ...(sel.length ? [{ label: `Folder z zaznaczonych (${sel.length})`, icon: 'folderPlus', run: () => newFolderWith(tab) }] : []),
        { sep: true },
        { label: 'Plik tekstowy', icon: 'filePlus', run: () => newFile(tab) },
        { label: 'Dokumenty', icon: 'doc', sub: [
          { label: 'Dokument Word', icon: 'doc', run: () => newFile(tab, 'Nowy dokument.docx', 'Nowy dokument Word') },
          { label: 'Arkusz Excel', icon: 'sheet', run: () => newFile(tab, 'Nowy arkusz.xlsx', 'Nowy arkusz Excel') },
          { label: 'Prezentacja PowerPoint', icon: 'doc', run: () => newFile(tab, 'Nowa prezentacja.pptx', 'Nowa prezentacja') },
          { label: 'Notatka Markdown', icon: 'doc', run: () => newFile(tab, 'Notatka.md', 'Nowa notatka') },
          { label: 'Plik CSV', icon: 'sheet', run: () => newFile(tab, 'Dane.csv', 'Nowy plik CSV') },
        ] },
        { label: 'Pliki programisty', icon: 'code', sub: [
          { label: 'JSON', icon: 'code', run: () => newFile(tab, 'plik.json', 'Nowy plik JSON') },
          { label: 'HTML', icon: 'code', run: () => newFile(tab, 'index.html', 'Nowy plik HTML') },
          { label: 'Skrypt PowerShell', icon: 'terminal', run: () => newFile(tab, 'skrypt.ps1', 'Nowy skrypt') },
          { label: 'Skrypt wsadowy (.bat)', icon: 'terminal', run: () => newFile(tab, 'skrypt.bat', 'Nowy skrypt') },
          { label: 'Inny plik…', icon: 'filePlus', run: () => newFile(tab, 'nowy plik', 'Nowy plik (z dowolnym rozszerzeniem)') },
        ] },
        ...(sel.length ? [{ sep: true } as const, { label: `Archiwum ZIP z zaznaczonych (${sel.length})`, icon: 'archive', run: () => zipSelection(tab) }] : []),
        { sep: true },
        { label: 'Synchronizację tego folderu…', icon: 'sync', run: () => (app.dialog = { type: 'sync', profile: null, source: tab.path }) },
      ],
    }
  }

  function setView(v: Tab['view']) {
    tab.view = v
    savePrefs(tab)
  }

  const FILTERS: [KindFilter, string, IconName][] = [['all', 'Wszystko', 'filter'], ['media', 'Media (zdjęcia i wideo)', 'gallery'], ['image', 'Zdjęcia', 'image'], ['video', 'Wideo', 'video'], ['doc', 'Dokumenty', 'doc'], ['audio', 'Audio', 'music'], ['archive', 'Archiwa', 'archive']]

  // Kategorie tylko te, które są w folderze; „Wszystko” zawsze.
  const shownFilters = $derived(FILTERS.filter(([k]) => k === 'all' || (tab.kindCounts[k] ?? 0) > 0 && !(k === 'media' && (!tab.kindCounts.image || !tab.kindCounts.video))))
  // Pasek faset otwiera się sam w galerii i po wybraniu kategorii; można go też przełączyć ręcznie.
  let facetsOpen = $state<boolean | null>(null)
  $effect(() => {
    tab.path
    facetsOpen = null
  })
  const facetsShown = $derived(tab.facetList.length > 0 && (facetsOpen ?? (tab.view === 'gallery' || tab.kindFilter !== 'all' || tab.facets.size > 0)))
  const facetGroups = $derived.by(() => {
    const m = new Map<string, typeof tab.facetList>()
    for (const f of tab.facetList) m.set(f.group, [...(m.get(f.group) ?? []), f])
    // Wybrane fasety, które zniknęły z listy, nadal muszą dać się odznaczyć.
    return [...m.entries()]
  })
  function toggleFacet(id: string) {
    if (tab.facets.has(id)) tab.facets.delete(id)
    else tab.facets.add(id)
  }

  function crumbDrop(e: DragEvent, path: string) {
    e.preventDefault()
    over = null
    if (app.dragging) transfer(app.dragging.paths, path, dropMode(app.dragging.paths, path, e))
  }
</script>

<div class="toolbar" class:dim={!focused}>
  <div class="row nav">
    <button class="icon-btn" title="Wstecz (Alt+←)" disabled={tab.hIndex <= 0 && !tab.search} onclick={() => back(tab)}><Icon name="back" /></button>
    <button class="icon-btn" title="Dalej (Alt+→)" disabled={tab.hIndex >= tab.history.length - 1} onclick={() => forward(tab)}><Icon name="fwd" /></button>
    <button class="icon-btn" title="W górę (Alt+↑)" disabled={tab.special} onclick={() => up(tab)}><Icon name="up" /></button>
    <button class="icon-btn" title="Odśwież (F5)" onclick={() => load(tab, true)}><Icon name="refresh" size={17} /></button>

    <div class="address" class:editing>
      {#if editing}
        <input
          bind:this={addrInput}
          bind:value={addr}
          spellcheck="false"
          oninput={suggest}
          onkeydown={addrKey}
          onblur={() => setTimeout(() => { editing = false; sugg = [] }, 150)}
        />
        {#if sugg.length}
          <div class="sugg fade-in">
            {#each sugg as s, i (s.path)}
              <button class:on={i === suggIndex} onmousedown={(e) => { e.preventDefault(); commit(s.path) }}>
                <Icon name="folder" size={15} /><span class="ellipsis">{s.path}</span>
              </button>
            {/each}
          </div>
        {/if}
      {:else}
        <div class="crumbs" role="button" tabindex="-1" onclick={(e) => e.target === e.currentTarget && startEdit()} onkeydown={() => {}}>
          {#if tab.special}
            <span class="crumb static">{tab.title}</span>
          {:else}
            {#key tab.path}<img class="cur" src={thumbUrl(tab.path, 16, 'i')} alt="" onerror={(e) => ((e.currentTarget as HTMLImageElement).style.visibility = 'hidden')} />{/key}
            {#each parts as c, i (c.path)}
              <button
                class="crumb"
                class:last={i === parts.length - 1}
                class:over={over === c.path}
                onclick={() => navigate(tab, c.path)}
                ondragover={(e) => { if (app.dragging) { e.preventDefault(); over = c.path } }}
                ondragleave={() => (over = null)}
                ondrop={(e) => crumbDrop(e, c.path)}
              >{c.name}</button>
              <button class="sep" title="Podfoldery" onclick={(e) => crumbMenu(e, c.path)}><Icon name="chevron" size={13} /></button>
            {/each}
            {#if tab.search}<span class="crumb static">wyniki wyszukiwania</span>{/if}
          {/if}
          <span class="fill" role="presentation" ondblclick={startEdit} onclick={startEdit}></span>
        </div>
      {/if}
    </div>

    <div class="search" class:active={!!q || !!tab.search}>
      <Icon name="search" size={16} />
      <input
        bind:this={searchInput}
        bind:value={q}
        placeholder={tab.special ? 'Szukaj…' : 'Filtruj · Enter: szukaj w podfolderach'}
        spellcheck="false"
        disabled={tab.special}
        onkeydown={onSearchKey}
        onfocus={() => (searchFocus = true)}
        onblur={() => setTimeout(() => (searchFocus = false), 150)}
        aria-describedby="search-help"
      />
      {#if tab.search && !tab.search.running && !savedHere}
        <button class="clear" title="Zapisz wyszukiwanie w panelu bocznym" aria-label="Zapisz wyszukiwanie" onclick={() => saveSearch(tab)}><Icon name="star" size={14} /></button>
      {/if}
      {#if searchFocus && !tab.special}
        <div class="qhelp fade-in" id="search-help" role="listbox" aria-label="Filtry wyszukiwania">
          {#if liveChips.length}
            <div class="qrow"><span class="faint">Rozpoznane:</span>{#each liveChips as c (c)}<span class="chip on">{c}</span>{/each}</div>
          {/if}
          <div class="qhint faint">Pisz, aby filtrować ten folder · <b>Enter</b> szuka także w podfolderach · dodaj filtr:</div>
          <div class="qrow wrap">
            {#each QUERY_HINTS as [h, l] (h)}
              <button class="chip" role="option" aria-selected="false" title={l} onmousedown={(e) => { e.preventDefault(); addHint(h) }}>{h}</button>
            {/each}
          </div>
        </div>
      {/if}
      {#if tab.search?.running}<span class="dot busy"></span>{/if}
      {#if q || tab.search}
        <button class="clear" title="Wyczyść (Esc)" onclick={() => { q = ''; tab.filter = ''; if (tab.search) closeSearch(tab) }}><Icon name="close" size={13} /></button>
      {/if}
    </div>
  </div>

  {#if !tab.special}
    <div class="row cmds">
      <button class="btn primary sm" onclick={newMenu}><Icon name="plus" size={16} />Nowy</button>
      <span class="vsep"></span>
      <button class="icon-btn" title="Wytnij (Ctrl+X)" disabled={!sel.length} onclick={() => copySel(tab, true)}><Icon name="cut" /></button>
      <button class="icon-btn" title="Kopiuj (Ctrl+C)" disabled={!sel.length} onclick={() => copySel(tab, false)}><Icon name="copy" /></button>
      <button class="icon-btn" title="Wklej (Ctrl+V)" onclick={() => paste(tab)}><Icon name="paste" /></button>
      <button class="icon-btn" title={sel.length > 1 ? 'Zmień nazwy (F2)' : 'Zmień nazwę (F2)'} disabled={!sel.length} onclick={() => (sel.length === 1 ? (tab.renaming = sel[0].path) : (app.dialog = { type: 'bulkRename', entries: sel }))}><Icon name="edit" /></button>
      <button class="icon-btn" title="Do Kosza (Del) · Shift+Del: trwale" disabled={!sel.length} onclick={(e) => trash(tab, e.shiftKey)}><Icon name="trash" /></button>
      <button class="icon-btn" title="Właściwości (Alt+Enter)" disabled={!sel.length} onclick={() => (app.dialog = { type: 'props', entries: sel })}><Icon name="info" /></button>
      <span class="grow"></span>
      {#if shownFilters.length > 2 || tab.facetList.length}
        <div class="filters" role="toolbar" aria-label="Filtry">
          {#if shownFilters.length > 2}
            {#each shownFilters as [k, label, icon] (k)}
              <button class="fchip" class:on={tab.kindFilter === k} title={k === 'all' ? label : `${label} (${tab.kindCounts[k]})`} aria-label={label} aria-pressed={tab.kindFilter === k} onclick={() => { tab.kindFilter = k; tab.facets.clear() }}><Icon name={icon} size={15} /></button>
            {/each}
          {/if}
          {#if tab.facetList.length}
            <button class="fchip more" class:on={facetsShown} class:active={tab.facets.size > 0} title="Filtry dopasowane do zawartości" aria-label="Więcej filtrów" aria-expanded={facetsShown} onclick={() => (facetsOpen = !facetsShown)}><Icon name="sort" size={15} />{#if tab.facets.size}<span class="fcount">{tab.facets.size}</span>{/if}</button>
          {/if}
        </div>
      {/if}
      <button class="btn ghost sm" onclick={sortMenu} title="Sortowanie"><Icon name="sort" size={16} />{SORT_LABEL[tab.sort]}{#if tab.group !== 'none'}<span class="faint"> · {GROUP_LABEL[tab.group]}</span>{/if}</button>
      {#if tab.view !== 'list'}
        <input
          class="zoom"
          type="range"
          min={tab.view === 'gallery' ? 120 : 84}
          max={tab.view === 'gallery' ? 420 : 280}
          step="4"
          title="Wielkość miniatur (Ctrl+kółko)"
          value={tab.view === 'gallery' ? settings.galleryRow : settings.gridSize}
          oninput={(e) => {
            const v = +(e.target as HTMLInputElement).value
            if (tab.view === 'gallery') settings.galleryRow = v
            else settings.gridSize = v
          }}
        />
      {/if}
      <div class="seg">
        <button class:on={tab.view === 'list'} title="Lista (Ctrl+1)" onclick={() => setView('list')}><Icon name="list" size={17} /></button>
        <button class:on={tab.view === 'grid'} title="Siatka (Ctrl+2)" onclick={() => setView('grid')}><Icon name="grid" size={16} /></button>
        <button class:on={tab.view === 'gallery'} title="Galeria (Ctrl+3)" onclick={() => setView('gallery')}><Icon name="gallery" size={16} /></button>
      </div>
      <button class="icon-btn" class:on={app.split} title="Podział na dwa panele (Ctrl+\)" onclick={toggleSplit}><Icon name="split" /></button>
      <button class="icon-btn" class:on={settings.shelfOpen} title="Schowek plików (Ctrl+Shift+S)" aria-label="Schowek plików" onclick={() => (settings.shelfOpen = !settings.shelfOpen)}><Icon name="archive" />{#if app.shelf.length}<span class="bubble">{app.shelf.length}</span>{/if}</button>
      <button class="icon-btn" class:on={settings.terminalOpen} title="Terminal (Ctrl+`)" aria-label="Terminal" onclick={() => (settings.terminalOpen = !settings.terminalOpen)}><Icon name="terminal" /></button>
      <button class="icon-btn" class:on={settings.previewPane} title="Panel podglądu (Ctrl+P)" onclick={() => (settings.previewPane = !settings.previewPane)}><Icon name="panel" /></button>
    </div>
    {#if facetsShown}
      <div class="facets fade-in" role="toolbar" aria-label="Filtry zawartości">
        {#each facetGroups as [g, list] (g)}
          <span class="fg">{g}</span>
          {#each list as f (f.id)}
            <button class="chip" class:on={tab.facets.has(f.id)} aria-pressed={tab.facets.has(f.id)} onclick={() => toggleFacet(f.id)}>{f.label}<span class="fc">{f.count}</span></button>
          {/each}
        {/each}
        {#if tab.facets.size}<span class="fsep"></span><button class="chip clear" title="Wyłącz wszystkie filtry" onclick={() => tab.facets.clear()}><Icon name="close" size={12} />Wyczyść filtry</button><span class="faint fres">{tab.items.length} wyników</span>{/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .toolbar {
    padding: 8px 12px 6px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: none;
  }
  .toolbar.dim {
    opacity: 0.72;
  }
  .nav {
    gap: 2px;
  }
  .address {
    position: relative;
    flex: 1;
    min-width: 0;
    height: 36px;
    margin: 0 6px;
    background: var(--high);
    border-radius: var(--r);
    display: flex;
    align-items: center;
  }
  .address.editing {
    background: var(--higher);
  }
  .address input {
    width: 100%;
    height: 100%;
    border: 0;
    background: transparent;
    padding: 0 14px;
    font-family: var(--font);
  }
  .crumbs {
    display: flex;
    align-items: center;
    height: 100%;
    width: 100%;
    padding: 0 6px;
    overflow: hidden;
    min-width: 0;
  }
  .cur {
    width: 16px;
    height: 16px;
    flex: none;
    margin: 0 4px 0 6px;
  }
  .crumb {
    height: 28px;
    padding: 0 8px;
    border-radius: var(--r-s);
    color: var(--dim);
    white-space: nowrap;
    flex: none;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .crumb.last {
    color: var(--text);
    font-weight: 600;
  }
  .crumb:not(.static):hover,
  .crumb.over {
    background: var(--press);
    color: var(--text);
  }
  .crumb.static {
    display: flex;
    align-items: center;
    color: var(--text);
    font-weight: 600;
  }
  .sep {
    width: 18px;
    height: 26px;
    border-radius: var(--r-xs);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--faint);
    flex: none;
  }
  .sep:hover {
    background: var(--press);
    color: var(--text);
  }
  .crumbs > .sep:nth-last-child(2) {
    display: none;
  }
  .fill {
    flex: 1;
    align-self: stretch;
    min-width: 30px;
  }
  .sugg {
    position: absolute;
    top: 40px;
    left: 0;
    right: 0;
    z-index: 50;
    background: var(--popup-bg);
    backdrop-filter: var(--popup-blur);
    border: 1px solid var(--line);
    border-radius: var(--r);
    padding: 6px;
    box-shadow: 0 20px 50px var(--shadow);
  }
  .sugg button {
    width: 100%;
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 7px 10px;
    border-radius: var(--r-s);
    color: var(--dim);
    text-align: left;
  }
  .sugg button:hover,
  .sugg button.on {
    background: var(--press);
    color: var(--text);
  }
  .search {
    width: 300px;
    height: 36px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 12px;
    background: var(--high);
    border-radius: var(--r);
    color: var(--faint);
    transition: width 0.2s var(--ease);
  }
  .search:focus-within,
  .search.active {
    width: 380px;
    background: var(--higher);
    color: var(--text);
  }
  .search {
    position: relative;
  }
  .qhelp {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    width: 360px;
    z-index: 60;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-radius: var(--r-l);
    background: var(--popup-bg);
    backdrop-filter: var(--popup-blur);
    border: 1px solid var(--line);
    box-shadow: 0 20px 50px var(--shadow);
  }
  .qrow {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
  }
  .qrow.wrap {
    flex-wrap: wrap;
  }
  .qrow .chip {
    font-family: var(--mono);
    font-size: 11px;
  }
  .qhint {
    font-size: 12px;
  }
  .qhint b {
    color: var(--text);
    font-weight: 600;
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    padding: 0;
  }
  .clear {
    width: 22px;
    height: 22px;
    border-radius: var(--r-xs);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--dim);
  }
  .clear:hover {
    background: var(--press);
  }
  .cmds {
    gap: 2px;
    min-height: 34px;
  }
  .btn.sm {
    height: 30px;
    padding: 0 12px 0 9px;
    gap: 6px;
  }
  .vsep {
    width: 1px;
    height: 18px;
    background: var(--line);
    margin: 0 6px;
  }
  .filters {
    display: flex;
    gap: 4px;
    overflow: hidden;
    min-width: 0;
  }
  .seg {
    display: flex;
    background: var(--high);
    border-radius: var(--r);
    padding: 2px;
    margin: 0 4px;
  }
  .seg button {
    width: 32px;
    height: 28px;
    border-radius: var(--r-s);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--faint);
  }
  .seg button:hover {
    color: var(--text);
  }
  .seg button.on {
    background: var(--accent);
    color: var(--on-accent);
  }
  .zoom {
    width: 96px;
    padding: 0;
    border: 0;
    background: transparent;
    accent-color: var(--accent);
  }
  @media (max-width: 1100px) {
    .filters {
      display: none;
    }
  }
  .fchip {
    width: 30px;
    height: 28px;
    border-radius: var(--r-s);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--faint);
  }
  .fchip:hover {
    background: var(--hover);
    color: var(--text);
  }
  .icon-btn {
    position: relative;
  }
  .bubble {
    position: absolute;
    top: 2px;
    right: 2px;
    min-width: 15px;
    height: 15px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 10px;
    font-weight: 700;
    line-height: 15px;
    text-align: center;
  }
  .fchip.more {
    position: relative;
    width: auto;
    min-width: 30px;
    padding: 0 6px;
    gap: 4px;
  }
  .fchip.active:not(.on) {
    color: var(--text);
    background: var(--sel);
  }
  .fcount {
    font-size: 11px;
    font-weight: 600;
  }
  .facets {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
    padding: 2px 2px 0;
  }
  .fg {
    margin: 0 2px 0 8px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--ghost);
  }
  .fg:first-child {
    margin-left: 2px;
  }
  .chip .fc {
    font-size: 11px;
    opacity: 0.55;
    font-variant-numeric: tabular-nums;
  }
  .chip.clear {
    background: transparent;
    outline: 1px solid var(--line-strong);
    outline-offset: -1px;
  }
  .fsep {
    width: 1px;
    height: 18px;
    margin: 0 6px;
    background: var(--line-strong);
  }
  .fres {
    font-size: 12px;
    margin-left: 4px;
  }
  .fchip.on {
    background: var(--accent);
    color: var(--on-accent);
  }
</style>
