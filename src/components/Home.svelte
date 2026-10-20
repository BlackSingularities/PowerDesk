<script lang="ts" module>
  import type { DirSize as DS } from '../lib/api'
  const sizeCache: Record<string, DS> = {}
  let sizeAt = 0
</script>

<script lang="ts">
  import Icon from './Icon.svelte'
  import Thumb from './Thumb.svelte'
  import {
    app, settings, navigate, newTab, addFavorite, removeFavorite, refreshDrives, toast, driveShown, showDriveMenu, ejectable, ejectDrive, lowSpace,
    toggleHomeDir, isHomeDir, removeRecent, toggleRecentPin, clearRecent, removeNetPlace, startSearch, openSpace, showViewer, SYNC, HOME, type Tab,
  } from '../lib/state.svelte'
  import { size, ago, basename, samePath, collator, isMedia, kindOf, parent, date } from '../lib/util'
  import { api, thumbUrl, type Drive, type Entry, type SystemInfo, type DirSize } from '../lib/api'

  let { tab }: { tab: Tab } = $props()

  const used = (d: Drive) => (d.total ? 1 - d.free / d.total : 0)
  const go = (p: string, e?: MouseEvent) => (e?.ctrlKey || e?.button === 1 ? newTab(p, false, tab.id) : navigate(tab, p))
  const isPinned = (p: string) => settings.recentPinned.some((r) => samePath(r, p))
  const recent = $derived(settings.showRecent ? [...settings.recentPinned, ...app.recent.filter((r) => !isPinned(r)).slice(0, 10)] : [])
  const open = (k: string) => !settings.collapsed['home:' + k]
  const toggle = (k: string) => (settings.collapsed['home:' + k] = !settings.collapsed['home:' + k])

  // Jak w Eksploratorze: klik zaznacza, dwuklik (albo Enter) otwiera; Ctrl/środkowy – nowa karta.
  let sel = $state<string | null>(null)
  const isFav = (p: string) => app.favorites.some((f) => f.path === p)
  function click(e: MouseEvent, p: string, ok = true) {
    if (e.ctrlKey) return ok && go(p, e)
    sel = p
    if (settings.singleClickOpen && ok) go(p)
  }
  const dbl = (p: string, ok = true) => ok && !settings.singleClickOpen && go(p)
  const key = (e: KeyboardEvent, p: string, ok = true) => e.key === 'Enter' && ok && go(p, e as unknown as MouseEvent)
  const aux = (e: MouseEvent, p: string, ok = true) => e.button === 1 && ok && newTab(p, false, tab.id)

  function native(paths: string[], e: MouseEvent) {
    api.nativeMenu(paths, e.clientX, e.clientY, e.shiftKey).then((done) => done && setTimeout(refreshDrives, 600)).catch((err) => toast(String(err), 'error'))
  }

  function folderMenu(e: MouseEvent, p: string) {
    e.preventDefault()
    sel = p
    if (e.shiftKey) return native([p], e)
    app.menu = {
      x: e.clientX, y: e.clientY,
      items: [
        { label: 'Otwórz', icon: 'folder', kbd: 'Enter', run: () => go(p) },
        { label: 'Otwórz w nowej karcie', icon: 'plus', kbd: 'Ctrl+klik', run: () => newTab(p, false, tab.id) },
        { sep: true },
        { label: 'Kopiuj ścieżkę', icon: 'link', run: () => { navigator.clipboard.writeText(p); toast('Skopiowano ścieżkę') } },
        isFav(p) ? { label: 'Odepnij', icon: 'pin', run: () => removeFavorite(p) } : { label: 'Przypnij w panelu bocznym', icon: 'pin', run: () => addFavorite(p) },
        { label: isHomeDir(p) ? 'Usuń z ekranu Komputer' : 'Pokaż zawartość na ekranie Komputer', icon: 'grid', run: () => toggleHomeDir(p) },
        { label: 'Terminal tutaj', icon: 'terminal', run: () => api.terminal(p) },
        { label: 'Pokaż w Eksploratorze', icon: 'open', run: () => api.open(p) },
        { sep: true },
        { label: 'Właściwości systemowe', icon: 'info', run: () => api.systemProps(p) },
        { label: 'Pokaż więcej opcji (menu Windows)', icon: 'more', kbd: 'Shift+PPM', run: () => native([p], e) },
      ],
    }
  }

  function driveMenu(e: MouseEvent, d: Drive) {
    sel = d.path
    showDriveMenu(e, d, (p) => go(p), (p) => newTab(p, false, tab.id))
  }

  // ---------- Dashboard ----------

  let now = $state(new Date())
  $effect(() => {
    const t = setInterval(() => (now = new Date()), 15_000)
    return () => clearInterval(t)
  })
  const greeting = $derived(now.getHours() < 5 ? 'Dobranoc' : now.getHours() < 12 ? 'Dzień dobry' : now.getHours() < 18 ? 'Miłego popołudnia' : 'Dobry wieczór')
  const today = $derived(now.toLocaleDateString('pl', { weekday: 'long', day: 'numeric', month: 'long' }))
  const clock = $derived(now.toLocaleTimeString('pl', { hour: '2-digit', minute: '2-digit' }))

  // Pamięć: wszystkie dyski lokalne razem + udział każdego w pierścieniu.
  const local = $derived(app.drives.filter((d) => d.online && d.total && d.kind !== 'network' && d.kind !== 'cdrom'))
  const totalAll = $derived(local.reduce((a, d) => a + d.total, 0))
  const usedAll = $derived(local.reduce((a, d) => a + d.total - d.free, 0))
  const RING = ['#4f8cff', '#22c55e', '#f59e0b', '#ec4899', '#8b5cf6', '#14b8a6', '#ef4444', '#eab308']
  const ring = $derived.by(() => {
    let acc = 0
    return local.map((d, i) => {
      const part = (d.total - d.free) / (totalAll || 1)
      const seg = { d, from: acc, part, color: RING[i % RING.length] }
      acc += d.total / (totalAll || 1)
      return seg
    })
  })
  let hoverDrive = $state<string | null>(null)
  /** Łuk pierścienia od ułamka `a` do `b` obwodu (0 = góra, zgodnie z ruchem wskazówek). */
  function arc(a: number, b: number, r = 52) {
    const gap = 0.004
    b = Math.max(a + 0.002, b - gap)
    const p = (t: number) => [64 + r * Math.cos(t * 2 * Math.PI - Math.PI / 2), 64 + r * Math.sin(t * 2 * Math.PI - Math.PI / 2)]
    const [x0, y0] = p(a), [x1, y1] = p(b)
    return `M${x0.toFixed(2)} ${y0.toFixed(2)} A${r} ${r} 0 ${b - a > 0.5 ? 1 : 0} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`
  }

  // Szukanie wszędzie: domyślnie w folderze domowym, można wybrać dysk.
  let q = $state('')
  let qRoot = $state('')
  $effect(() => {
    if (!qRoot) qRoot = app.known.find((k) => k.id === 'home')?.path ?? local[0]?.path ?? ''
  })
  function searchAll() {
    if (!q.trim() || !qRoot) return
    const query = q
    navigate(tab, qRoot)
    queueMicrotask(() => startSearch(tab, query))
  }

  // Ostatnie pliki z Pulpitu, Pobranych, Dokumentów, Obrazów i Wideo.
  let recentFiles = $state<Entry[] | null>(null)
  let fileKind = $state<'all' | 'image' | 'doc' | 'video'>('all')
  async function loadRecentFiles() {
    const dirs = app.known.filter((k) => ['desktop', 'downloads', 'documents', 'pictures', 'videos'].includes(k.id)).map((k) => k.path)
    if (!dirs.length) return
    recentFiles = await api.recentFiles(dirs, 14, 400).catch(() => [])
  }
  $effect(() => {
    if (app.known.length && recentFiles === null) loadRecentFiles()
  })
  const shownFiles = $derived(
    (recentFiles ?? []).filter((e) => {
      const k = kindOf(e)
      return fileKind === 'all' || (fileKind === 'image' ? k === 'image' || k === 'raw' : fileKind === 'video' ? k === 'video' : ['pdf', 'doc', 'sheet', 'slides', 'text'].includes(k))
    }).slice(0, 24),
  )
  const downloads = $derived((recentFiles ?? []).filter((e) => samePath(parent(e.path) ?? '', app.known.find((k) => k.id === 'downloads')?.path ?? '')).slice(0, 5))
  const running = $derived(app.ops.filter((o) => o.state === 'running'))
  const syncBusy = $derived(app.syncProfiles.filter((p) => ['syncing', 'scanning'].includes(app.syncStatus[p.id]?.state ?? '')).length)
  const syncOff = $derived(app.syncProfiles.filter((p) => !app.syncStatus[p.id]?.connected).length)

  // ---------- Szczegóły: system, foldery, aktywność ----------

  let sys = $state<SystemInfo | null>(null)
  // Historia zużycia procesora – mały wykres z ostatnich ~2 minut.
  let cpuHist = $state<number[]>([])
  $effect(() => {
    let alive = true
    const tick = () => api.systemInfo().then((v) => { if (!alive) return; sys = v; cpuHist = [...cpuHist.slice(-39), v.cpu_usage] }).catch(() => {})
    tick()
    const t = setInterval(() => !document.hidden && tick(), 3000)
    return () => {
      alive = false
      clearInterval(t)
    }
  })
  const uptime = (s: number) => {
    const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60)
    return d ? `${d} d ${h} h` : h ? `${h} h ${m} min` : `${m} min`
  }
  // Historia zużycia procesora – mały wykres z ostatnich ~2 minut.
  
  const spark = (vals: number[], w = 160, h = 36) =>
    vals.length < 2 ? '' : vals.map((v, i) => `${i ? 'L' : 'M'}${((i / (vals.length - 1)) * w).toFixed(1)} ${(h - (Math.min(100, v) / 100) * h).toFixed(1)}`).join(' ')

  // Rozmiary folderów użytkownika – liczone raz w tle i pamiętane między wejściami na ekran.
  let folderSizes = $state<Record<string, DirSize>>(sizeCache)
  $effect(() => {
    if (!app.known.length) return
    const ids = ['downloads', 'documents', 'pictures', 'videos', 'music', 'desktop']
    const todo = app.known.filter((k) => ids.includes(k.id) && !sizeCache[k.path])
    if (!todo.length || Date.now() - sizeAt < 10 * 60_000 && Object.keys(sizeCache).length) return
    sizeAt = Date.now()
    todo.reduce((acc, k) => acc.then(() => api.dirSize(k.path).then((d) => { sizeCache[k.path] = d; folderSizes = { ...sizeCache } })), Promise.resolve())
  })
  const userFolders = $derived(
    app.known.filter((k) => folderSizes[k.path]).map((k) => ({ k, s: folderSizes[k.path] })).sort((a, b) => b.s.size - a.s.size),
  )
  const maxFolder = $derived(Math.max(1, ...userFolders.map((f) => f.s.size)))

  // Ile plików zmieniono każdego dnia w ostatnich 14 dniach.
  const days = $derived.by(() => {
    const out: { d: Date; n: number; bytes: number }[] = []
    const start = new Date(now.getFullYear(), now.getMonth(), now.getDate())
    for (let i = 13; i >= 0; i--) out.push({ d: new Date(start.getTime() - i * 86_400_000), n: 0, bytes: 0 })
    for (const e of recentFiles ?? []) {
      const i = Math.floor((e.modified - out[0].d.getTime()) / 86_400_000)
      if (i >= 0 && i < 14) {
        out[i].n++
        out[i].bytes += e.size
      }
    }
    return out
  })
  const maxDay = $derived(Math.max(1, ...days.map((d) => d.n)))
  let hoverDay = $state<number | null>(null)
  const weekTotal = $derived(days.slice(7).reduce((a, d) => a + d.n, 0))
  const kindsWeek = $derived.by(() => {
    const m: Record<string, number> = {}
    for (const e of recentFiles ?? []) {
      const k = kindOf(e)
      const g = k === 'image' || k === 'raw' ? 'Zdjęcia' : k === 'video' ? 'Wideo' : ['pdf', 'doc', 'sheet', 'slides', 'text'].includes(k) ? 'Dokumenty' : k === 'archive' ? 'Archiwa' : 'Inne'
      m[g] = (m[g] ?? 0) + 1
    }
    return Object.entries(m).sort((a, b) => b[1] - a[1])
  })

  const ACTIONS: { label: string; icon: import('../lib/icons').IconName; run: () => void; kbd?: string }[] = [
    { label: 'Paleta poleceń', icon: 'bolt', kbd: 'Ctrl+K', run: () => (app.palette = true) },
    { label: 'Nowa karta', icon: 'plus', kbd: 'Ctrl+T', run: () => newTab(HOME, true, tab.id) },
    { label: 'Pobrane', icon: 'download', run: () => { const d = app.known.find((k) => k.id === 'downloads'); if (d) go(d.path) } },
    { label: 'Zajętość dysku', icon: 'grid', run: () => openSpace(tab, local[0]?.path ?? 'C:\\') },
    { label: 'Porządki', icon: 'trash', run: () => openSpace(tab, app.known.find((k) => k.id === 'home')?.path ?? 'C:\\') },
    { label: 'Połącz z siecią', icon: 'network', run: () => (app.dialog = { type: 'network' }) },
    { label: 'Schowek plików', icon: 'archive', kbd: 'Ctrl+⇧+S', run: () => (settings.shelfOpen = !settings.shelfOpen) },
    { label: 'Terminal', icon: 'terminal', kbd: 'Ctrl+`', run: () => (settings.terminalOpen = !settings.terminalOpen) },
  ]

  // ---------- Sekcje: kolejność i przeciąganie ----------

  const BASE = ['overview', 'insights', 'folders', 'files', 'drives', 'removable', 'network', 'sync', 'recent']
  const TITLES: Record<string, string> = {
    overview: 'Przegląd', insights: 'Szczegóły', files: 'Ostatnie pliki', folders: 'Foldery', drives: 'Dyski', removable: 'Urządzenia wymienne', network: 'Lokalizacje sieciowe', sync: 'Synchronizacja', recent: 'Ostatnio otwierane',
  }
  const order = $derived.by(() => {
    const want = [...BASE, ...settings.homeDirs.map((p) => 'dir:' + p)]
    const kept = settings.homeOrder.filter((id) => want.includes(id))
    // Nowe sekcje: bazowe na koniec, foldery z zawartością przed „Ostatnio otwierane”.
    for (const id of want) {
      if (kept.includes(id)) continue
      const at = id.startsWith('dir:') ? kept.indexOf('recent') : -1
      if (id === 'overview') kept.unshift(id)
      else if (id === 'insights') kept.splice(kept.indexOf('overview') + 1, 0, id)
      else if (id === 'files') kept.splice(Math.max(0, kept.indexOf('folders') + 1), 0, id)
      else if (at >= 0) kept.splice(at, 0, id)
      else kept.push(id)
    }
    return kept
  })
  const titleOf = (id: string) => TITLES[id] ?? (basename(id.slice(4)) || id.slice(4))
  const driveList = (id: string) =>
    id === 'drives' ? app.drives.filter((d) => d.kind === 'fixed' || d.kind === 'ram' || d.kind === 'unknown')
    : id === 'removable' ? app.drives.filter((d) => (d.kind === 'removable' || d.kind === 'cdrom') && driveShown(d))
    : app.drives.filter((d) => d.kind === 'network')
  function shown(id: string) {
    if (id === 'files') return settings.showRecent
    if (id === 'removable') return driveList(id).length > 0
    if (id === 'sync') return app.syncProfiles.length > 0
    if (id === 'recent') return recent.length > 0
    return true
  }

  let dragSec = $state<string | null>(null)
  let dropAt = $state<{ id: string; before: boolean } | null>(null)
  function secStart(e: DragEvent, id: string) {
    dragSec = id
    e.dataTransfer!.effectAllowed = 'move'
    e.dataTransfer!.setData('text/ofp-section', id)
  }
  function secOver(e: DragEvent, id: string) {
    if (!dragSec) return
    e.preventDefault()
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    dropAt = id === dragSec ? null : { id, before: e.clientY < r.top + r.height / 2 }
  }
  function secDrop(e: DragEvent, id: string) {
    e.preventDefault()
    if (dragSec && dropAt && dragSec !== id) {
      const list = order.filter((x) => x !== dragSec)
      list.splice(list.indexOf(id) + (dropAt.before ? 0 : 1), 0, dragSec)
      settings.homeOrder = list
    }
    secEnd()
  }
  function secEnd() {
    dragSec = null
    dropAt = null
  }

  // ---------- Foldery pokazywane z zawartością ----------

  let dirs = $state<Record<string, Entry[] | undefined>>({})
  $effect(() => {
    for (const p of settings.homeDirs) {
      if (dirs[p]) continue
      api.listDir(p)
        .then((l) => {
          const vis = l.filter((e) => settings.showHidden || !(e.hidden || e.system))
          // Najpierw foldery, potem najnowsze pliki – to zwykle to, po co się wraca.
          vis.sort((a, b) => (a.dir !== b.dir ? (a.dir ? -1 : 1) : a.dir ? collator.compare(a.name, b.name) : b.modified - a.modified))
          dirs[p] = vis
        })
        .catch(() => (dirs[p] = []))
    }
  })
  function openItem(e: Entry, list: Entry[]) {
    if (e.dir) return go(e.path)
    if (isMedia(e)) {
      const media = list.filter(isMedia)
      showViewer(media, Math.max(0, media.findIndex((x) => x.path === e.path)))
    } else api.open(e.path).catch((err) => toast(String(err), 'error'))
  }
  function itemMenu(ev: MouseEvent, e: Entry) {
    if (e.dir) return folderMenu(ev, e.path)
    ev.preventDefault()
    sel = e.path
    native([e.path], ev)
  }
</script>

<div class="home" role="presentation" onmousedown={(e) => { if (!(e.target as HTMLElement).closest('button')) sel = null }}>
  {#each order as id (id)}
    {#if shown(id)}
      <section
        class:dragging={dragSec === id}
        class:drop-before={dropAt?.id === id && dropAt.before}
        class:drop-after={dropAt?.id === id && !dropAt.before}
        ondragover={(e) => secOver(e, id)}
        ondrop={(e) => secDrop(e, id)}
        role="group"
        aria-label={titleOf(id)}
      >
        <h2 class="section-title row head">
          <span class="grip" draggable="true" role="button" tabindex="-1" aria-label="Zmień kolejność sekcji" title="Przeciągnij, aby zmienić kolejność" ondragstart={(e) => secStart(e, id)} ondragend={secEnd}><Icon name="more" size={14} /></span>
          <button class="fold" aria-expanded={open(id)} onclick={() => toggle(id)}>
            <span class="chev" class:shut={!open(id)}><Icon name="chevronDown" size={13} /></span>
            <span class="ellipsis">{titleOf(id)}</span>
            {#if id.startsWith('dir:') && dirs[id.slice(4)]}<span class="cnt">{dirs[id.slice(4)]!.length}</span>{/if}
          </button>
          <span class="grow"></span>
          {#if id === 'network'}
            <button class="btn ghost sm" onclick={() => (app.dialog = { type: 'network' })}><Icon name="plus" size={15} />Połącz z lokalizacją sieciową</button>
          {:else if id === 'recent' && app.recent.length}
            <button class="btn ghost sm" onclick={clearRecent} title="Przypięte pozycje zostaną">Wyczyść historię</button>
          {:else if id.startsWith('dir:')}
            <button class="btn ghost sm" onclick={() => go(id.slice(4))}>Otwórz</button>
            <button class="icon-btn xs" title="Usuń z ekranu Komputer" aria-label="Usuń z ekranu Komputer" onclick={() => toggleHomeDir(id.slice(4))}><Icon name="close" size={13} /></button>
          {/if}
        </h2>
        {#if open(id)}
          {#if id === 'overview'}{@render overview()}
          {:else if id === 'insights'}{@render insights()}
          {:else if id === 'files'}{@render files()}
          {:else if id === 'folders'}{@render folders()}
          {:else if id === 'drives' || id === 'removable' || id === 'network'}{@render drives(id)}
          {:else if id === 'sync'}{@render syncs()}
          {:else if id === 'recent'}{@render recents()}
          {:else}{@render dirView(id.slice(4))}{/if}
        {/if}
      </section>
    {/if}
  {/each}
</div>

{#snippet overview()}
  <div class="dash">
    <div class="hero card">
      <div class="hello">
        <span class="faint cap1">{today}</span>
        <h1>{greeting}</h1>
        <span class="clock">{clock}</span>
      </div>
      <form class="qs" onsubmit={(e) => { e.preventDefault(); searchAll() }}>
        <Icon name="search" size={17} />
        <input bind:value={q} placeholder="Szukaj plików…" spellcheck="false" aria-label="Szukaj plików" />
        <select bind:value={qRoot} aria-label="Gdzie szukać">
          {#each app.known.filter((k) => k.id === 'home') as k (k.id)}<option value={k.path}>Folder domowy</option>{/each}
          {#each local as d (d.letter)}<option value={d.path}>{d.label || 'Dysk'} ({d.letter})</option>{/each}
        </select>
        <button class="btn primary sm" disabled={!q.trim()}>Szukaj</button>
      </form>
      {#if app.favorites.length || app.recent.length}
        <div class="jump">
          <span class="faint jl">Skocz do</span>
          {#each [...app.favorites.map((f) => f.path), ...app.recent].filter((p, i, a) => a.findIndex((x) => samePath(x, p)) === i).slice(0, 8) as p (p)}
            <button class="jc" onclick={(e) => go(p, e)} onauxclick={(e) => aux(e, p)} oncontextmenu={(e) => folderMenu(e, p)} title={p}><img src={thumbUrl(p, 16, 'i')} alt="" /><span class="ellipsis">{basename(p) || p}</span></button>
          {/each}
        </div>
      {/if}
      <div class="acts">
        {#each ACTIONS as a (a.label)}
          <button class="act" onclick={a.run} title={a.kbd ? `${a.label} (${a.kbd})` : a.label}><Icon name={a.icon} size={18} /><span>{a.label}</span></button>
        {/each}
      </div>
    </div>

    <div class="card storage">
      <h3>Pamięć</h3>
      <div class="ringrow">
        <svg viewBox="0 0 128 128" class="ring" role="img" aria-label="Zajętość wszystkich dysków">
          <circle cx="64" cy="64" r="52" class="track" />
          {#each ring as s (s.d.letter)}
            {#if s.part > 0}<path d={arc(s.from, s.from + s.part)} class="seg" class:dim={hoverDrive && hoverDrive !== s.d.letter} stroke={s.color} />{/if}
          {/each}
          <text x="64" y="60" class="big">{Math.round((usedAll / (totalAll || 1)) * 100)}%</text>
          <text x="64" y="78" class="sm">zajęte</text>
        </svg>
        <div class="legend">
          <b>{size(totalAll - usedAll)}</b><span class="faint">wolne z {size(totalAll)}</span>
          {#each ring as s (s.d.letter)}
            <button class="lg" onmouseenter={() => (hoverDrive = s.d.letter)} onmouseleave={() => (hoverDrive = null)} onclick={() => go(s.d.path)} oncontextmenu={(e) => driveMenu(e, s.d)} title="{s.d.label} – kliknij, aby otworzyć">
              <span class="sw" style:background={s.color}></span>
              <span class="ellipsis grow">{s.d.label || 'Dysk'} <span class="faint">{s.d.letter}</span></span>
              <span class="faint" class:low={lowSpace(s.d)}>{size(s.d.free)}</span>
            </button>
          {/each}
        </div>
      </div>
      <button class="btn ghost sm full" onclick={() => openSpace(tab, local[0]?.path ?? 'C:\\')}><Icon name="grid" size={14} />Co zajmuje miejsce?</button>
    </div>

    <div class="card activity">
      <h3 class="row"><span class="grow">Aktywność</span>{#if !running.length && !app.syncProfiles.length && !app.shelf.length && !app.update}<span class="faint calm"><Icon name="check" size={14} />Wszystko spokojnie</span>{/if}</h3>
      {#each running as o (o.id)}
        <div class="arow"><span class="dot busy"></span><span class="ellipsis grow">{o.title}</span><span class="faint">{Math.round((o.done_bytes / (o.total_bytes || 1)) * 100)}%</span></div>
        <span class="bar"><i style:width="{(o.done_bytes / (o.total_bytes || 1)) * 100}%"></i></span>
      {/each}
      {#if app.syncProfiles.length}
        <button class="arow lnk" onclick={() => navigate(tab, SYNC)}><span class="dot {syncBusy ? 'busy' : syncOff ? '' : 'ok'}"></span><span class="grow">Synchronizacja</span><span class="faint">{syncBusy ? 'trwa…' : syncOff ? `${syncOff} odłączone` : 'aktualna'}</span></button>
      {/if}
      {#if app.shelf.length}
        <button class="arow lnk" onclick={() => (settings.shelfOpen = true)}><Icon name="archive" size={14} /><span class="grow">Schowek plików</span><span class="faint">{app.shelf.length} elem.</span></button>
      {/if}
      {#if app.update}
        <div class="arow"><Icon name="download" size={14} /><span class="grow">Dostępna wersja {app.update.latest}</span></div>
      {/if}
      {#if downloads.length}
        <h4 class="faint">Ostatnio pobrane</h4>
        {#each downloads as e (e.path)}
          <button class="arow lnk" ondblclick={() => openItem(e, downloads)} onclick={() => (sel = e.path)} class:sel={sel === e.path} oncontextmenu={(ev) => itemMenu(ev, e)} title={e.path}>
            <span class="mini"><Thumb entry={e} px={24} iconOnly /></span><span class="ellipsis grow">{e.name}</span><span class="faint">{ago(e.modified)}</span>
          </button>
        {/each}
      {/if}
    </div>
  </div>
{/snippet}

{#snippet insights()}
  <div class="ins">
    <div class="card sys">
      <h3 class="row"><span class="grow">Ten komputer</span><span class="faint small">{sys?.host ?? ''}</span></h3>
      {#if sys}
        <p class="faint small os">{sys.os}</p>
        <div class="meter">
          <div class="mh"><span>Procesor</span><b>{Math.round(sys.cpu_usage)}%</b></div>
          <svg viewBox="0 0 160 36" preserveAspectRatio="none" class="spark" aria-hidden="true"><path d={spark(cpuHist)} /></svg>
          <span class="faint small ellipsis" title={sys.cpu}>{sys.cpu} · {sys.cores} rdz. / {sys.threads} wątków</span>
        </div>
        <div class="meter">
          <div class="mh"><span>Pamięć RAM</span><b>{size(sys.mem_used)} / {size(sys.mem_total)}</b></div>
          <span class="bar" class:warn={sys.mem_used / sys.mem_total > 0.8} class:full={sys.mem_used / sys.mem_total > 0.92}><i style:width="{(sys.mem_used / sys.mem_total) * 100}%"></i></span>
        </div>
        <div class="kv"><span class="faint">Działa od</span><b>{uptime(sys.uptime)}</b></div>
      {:else}
        <p class="faint small">Wczytywanie…</p>
      {/if}
    </div>

    <div class="card uf">
      <h3>Twoje foldery</h3>
      {#each userFolders as { k, s } (k.id)}
        <button class="ufr" onclick={(e) => go(k.path, e)} oncontextmenu={(e) => folderMenu(e, k.path)} title="{k.path} · {s.files.toLocaleString('pl')} plików">
          <img src={thumbUrl(k.path, 16, 'i')} alt="" />
          <span class="ufn">{k.name}</span>
          <span class="ufb"><i style:width="{(s.size / maxFolder) * 100}%"></i></span>
          <span class="faint ufs">{size(s.size)}</span>
        </button>
      {:else}
        <p class="faint small">Liczę rozmiary…</p>
      {/each}
    </div>

    <div class="card chart">
      <h3 class="row"><span class="grow">Aktywność plików</span><span class="faint small">{weekTotal} zmian w tym tygodniu</span></h3>
      <div class="bars" role="img" aria-label="Liczba zmienionych plików w ostatnich 14 dniach">
        {#each days as d, i (i)}
          <button class="bcol" class:today={i === 13} onmouseenter={() => (hoverDay = i)} onmouseleave={() => (hoverDay = null)} onclick={() => { fileKind = 'all' }} title="{d.d.toLocaleDateString('pl', { weekday: 'short', day: 'numeric', month: 'short' })}: {d.n} plików · {size(d.bytes)}">
            <i style:height="{Math.max(3, (d.n / maxDay) * 100)}%" class:empty={!d.n}></i>
            <span>{d.d.toLocaleDateString('pl', { weekday: 'narrow' })}</span>
          </button>
        {/each}
      </div>
      <p class="faint small hl">
        {#if hoverDay !== null}{days[hoverDay].d.toLocaleDateString('pl', { weekday: 'long', day: 'numeric', month: 'long' })}: <b>{days[hoverDay].n}</b> plików ({size(days[hoverDay].bytes)})
        {:else}{#each kindsWeek.slice(0, 4) as [g, n], i (g)}{i ? ' · ' : ''}{g} {n}{/each}{/if}
      </p>
    </div>
  </div>
{/snippet}

{#snippet files()}
  <div class="fbar">
    {#each [['all', 'Wszystkie'], ['image', 'Zdjęcia'], ['doc', 'Dokumenty'], ['video', 'Wideo']] as [k, l] (k)}
      <button class="chip" class:on={fileKind === k} onclick={() => (fileKind = k as typeof fileKind)}>{l}</button>
    {/each}
    <span class="grow"></span>
    <button class="icon-btn xs" title="Odśwież" aria-label="Odśwież ostatnie pliki" onclick={loadRecentFiles}><Icon name="refresh" size={14} /></button>
  </div>
  {#if recentFiles === null}
    <p class="faint small pad">Wczytywanie…</p>
  {:else if !shownFiles.length}
    <p class="faint small pad">Brak plików zmienionych w ostatnich dwóch tygodniach</p>
  {:else}
    <div class="dir">
      {#each shownFiles as e (e.path)}
        <button class="tile" class:sel={sel === e.path} onclick={() => (sel = e.path)} ondblclick={() => openItem(e, shownFiles)} onkeydown={(ev) => ev.key === 'Enter' && openItem(e, shownFiles)} oncontextmenu={(ev) => itemMenu(ev, e)} title="{e.name}&#10;{e.path}&#10;Zmieniono: {date(e.modified)}">
          <span class="tt"><Thumb entry={e} px={110} fit={isMedia(e) ? 'cover' : 'contain'} /></span>
          <span class="ellipsis tn">{e.name}</span>
          <span class="faint tn small">{ago(e.modified)}</span>
        </button>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet folders()}
  <div class="folders">
    {#each app.known as k (k.id)}
      <button class="folder" class:sel={sel === k.path} onclick={(e) => click(e, k.path)} ondblclick={() => dbl(k.path)} onkeydown={(e) => key(e, k.path)} onauxclick={(e) => aux(e, k.path)} oncontextmenu={(e) => folderMenu(e, k.path)}>
        <span class="fi"><img class="sys" src={thumbUrl(k.path, 24, 'i')} alt="" onerror={(e) => ((e.currentTarget as HTMLImageElement).style.display = 'none')} /></span>
        <span class="ellipsis">{k.name}</span>
      </button>
    {/each}
    {#each app.favorites as f (f.path)}
      <button class="folder" class:sel={sel === f.path} onclick={(e) => click(e, f.path)} ondblclick={() => dbl(f.path)} onkeydown={(e) => key(e, f.path)} onauxclick={(e) => aux(e, f.path)} oncontextmenu={(e) => folderMenu(e, f.path)} title={f.path}>
        <span class="fi"><img class="sys" src={thumbUrl(f.path, 24, 'i')} alt="" /></span>
        <span class="ellipsis">{f.name}</span>
      </button>
    {/each}
  </div>
{/snippet}

{#snippet drives(id: string)}
  <div class="drives">
    {#each driveList(id) as d (d.letter)}
      <button class="drive" class:sel={sel === d.path} class:off={!d.online} onclick={(e) => click(e, d.path, d.online)} ondblclick={() => (d.online ? dbl(d.path) : toast(`${d.letter} – niedostępny`))} onkeydown={(e) => key(e, d.path, d.online)} onauxclick={(e) => aux(e, d.path, d.online)} oncontextmenu={(e) => driveMenu(e, d)} title={d.remote ?? d.path}>
        <span class="di"><img class="sysbig" src={thumbUrl(d.path, 40, 'i')} alt="" /></span>
        <span class="grow">
          <span class="row"><b class="ellipsis grow">{d.label || (d.kind === 'network' ? d.remote?.split('\\').filter(Boolean).pop() : 'Dysk lokalny')}</b>{#if ejectable(d)}<span class="ej" role="button" tabindex="-1" title="Wysuń bezpiecznie" onclick={(e) => { e.stopPropagation(); ejectDrive(d) }} onkeydown={() => {}}><Icon name="eject" size={15} /></span>{/if}<span class="faint">{d.letter}</span></span>
          {#if d.online && d.total}
            <span class="bar" class:warn={used(d) > 0.8} class:full={used(d) > 0.9}><i style:width="{used(d) * 100}%"></i></span>
            <span class="faint small cap"><span class:low={lowSpace(d)}>{size(d.free)} wolne z {size(d.total)} · {d.fs}</span>
              {#if lowSpace(d)}<span class="clean" role="button" tabindex="-1" onclick={(e) => { e.stopPropagation(); api.diskCleanup(d.letter).catch((er) => toast(String(er), 'error')) }} onkeydown={() => {}}>Zwolnij miejsce…</span>{/if}
            </span>
          {:else if d.kind === 'network'}
            <span class="faint small ellipsis">{d.online ? d.remote : `rozłączony · ${d.remote}`}</span>
          {:else}
            <span class="faint small">Brak nośnika</span>
          {/if}
        </span>
      </button>
    {/each}
    {#if id === 'network'}
      {#each settings.netPlaces as p (p.id)}
        <button class="drive" class:sel={sel === p.path} onclick={(e) => click(e, p.path)} ondblclick={() => dbl(p.path)} onkeydown={(e) => key(e, p.path)} onauxclick={(e) => aux(e, p.path)} oncontextmenu={(e) => { e.preventDefault(); app.menu = { x: e.clientX, y: e.clientY, items: [{ label: 'Otwórz', icon: 'folder', run: () => go(p.path) }, { label: 'Otwórz w nowej karcie', icon: 'plus', run: () => newTab(p.path, false, tab.id) }, { sep: true }, { label: 'Usuń z listy', icon: 'unlink', run: () => removeNetPlace(p, false) }, { label: 'Usuń i zapomnij hasło', icon: 'unlink', danger: true, run: () => removeNetPlace(p, true) }] } }}>
          <span class="di"><Icon name="network" size={24} /></span>
          <span class="grow"><span class="row"><b class="ellipsis grow">{p.name}</b><span class="faint">FTP</span></span><span class="faint small ellipsis">ftp://{p.path.slice(6).replace(/\\/g, '/')}</span></span>
        </button>
      {/each}
    {/if}
    {#if id === 'network' && !driveList(id).length && !settings.netPlaces.length}
      <button class="drive add" onclick={() => (app.dialog = { type: 'network' })}>
        <span class="di"><Icon name="plus" size={22} /></span>
        <span class="grow"><b>Połącz z lokalizacją sieciową</b><span class="faint small" style="display:block">Udział Windows / NAS, FTP, SFTP lub WebDAV</span></span>
      </button>
    {/if}
  </div>
{/snippet}

{#snippet syncs()}
  <div class="drives">
    {#each app.syncProfiles as p (p.id)}
      {@const s = app.syncStatus[p.id]}
      <button class="drive" onclick={() => navigate(tab, SYNC)}>
        <span class="di"><Icon name="sync" size={24} /></span>
        <span class="grow">
          <span class="row"><span class="dot {s?.state === 'syncing' || s?.state === 'scanning' ? 'busy' : s?.state === 'error' ? 'rec' : s?.connected ? 'ok' : ''}"></span><b class="ellipsis grow">{p.name}</b></span>
          <span class="faint small ellipsis">{s?.connected ? `Połączono · ostatnia synchronizacja ${ago(s.last_sync)}` : `Cel odłączony · był dostępny ${ago(s?.last_seen ?? 0)}`}</span>
        </span>
      </button>
    {/each}
  </div>
{/snippet}

{#snippet recents()}
  <div class="recent">
    {#each recent as r (r)}
      {@const pin = isPinned(r)}
      <div class="rrow" class:sel={sel === r}>
        <button class="rmain" onclick={(e) => click(e, r)} ondblclick={() => dbl(r)} onkeydown={(e) => key(e, r)} onauxclick={(e) => aux(e, r)} oncontextmenu={(e) => folderMenu(e, r)} title={r}>
          <img class="sys16" src={thumbUrl(r, 16, 'i')} alt="" /><span class="ellipsis grow">{basename(r) || r}</span><span class="faint ellipsis path">{r}</span>
        </button>
        <button class="ra" class:on={pin} title={pin ? 'Odepnij' : 'Przypnij na górze listy'} aria-label={pin ? 'Odepnij' : 'Przypnij'} onclick={() => toggleRecentPin(r)}><Icon name="pin" size={14} /></button>
        <button class="ra" title="Usuń z historii" aria-label="Usuń z historii" onclick={() => removeRecent(r)}><Icon name="close" size={14} /></button>
      </div>
    {/each}
  </div>
{/snippet}

{#snippet dirView(path: string)}
  {@const list = dirs[path]}
  {#if !list}
    <p class="faint small pad">Wczytywanie…</p>
  {:else if !list.length}
    <p class="faint small pad">Folder jest pusty lub niedostępny</p>
  {:else}
    <div class="dir">
      {#each list.slice(0, 24) as e (e.path)}
        <button class="tile" class:sel={sel === e.path} onclick={(ev) => { sel = e.path; if (ev.ctrlKey && e.dir) go(e.path, ev) }} ondblclick={() => openItem(e, list)} onkeydown={(ev) => ev.key === 'Enter' && openItem(e, list)} oncontextmenu={(ev) => itemMenu(ev, e)} title={e.name}>
          <span class="tt"><Thumb entry={e} px={110} fit={isMedia(e) ? 'cover' : 'contain'} iconOnly={e.dir} /></span>
          <span class="ellipsis tn">{e.name}</span>
        </button>
      {/each}
      {#if list.length > 24}
        <button class="tile more" onclick={() => go(path)}><span class="tt"><b>+{list.length - 24}</b></span><span class="tn">Pokaż wszystko</span></button>
      {/if}
    </div>
  {/if}
{/snippet}

<style>
  .ins {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
  }
  @media (max-width: 1100px) {
    .ins {
      grid-template-columns: 1fr 1fr;
    }
  }
  .small {
    font-size: 12px;
  }
  .os {
    margin: -6px 0 0;
  }
  .meter {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .mh {
    display: flex;
    justify-content: space-between;
    font-size: 12.5px;
  }
  .mh b {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .spark {
    width: 100%;
    height: 36px;
  }
  .spark path {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.6;
    vector-effect: non-scaling-stroke;
  }
  .meter .bar {
    display: block;
    height: 6px;
  }
  .kv {
    display: flex;
    justify-content: space-between;
    font-size: 12.5px;
    margin-top: auto;
  }
  .ufr {
    display: grid;
    grid-template-columns: 16px 90px 1fr 70px;
    align-items: center;
    gap: 10px;
    height: 28px;
    padding: 0 6px;
    border-radius: var(--r-s);
    font-size: 12.5px;
    text-align: left;
  }
  .ufr:hover {
    background: var(--hover);
  }
  .ufr img {
    width: 16px;
    height: 16px;
  }
  .ufn {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ufb {
    height: 6px;
    border-radius: 3px;
    background: var(--high);
    overflow: hidden;
  }
  .ufb i {
    display: block;
    height: 100%;
    border-radius: 3px;
    background: var(--accent);
  }
  .ufs {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .bars {
    flex: 1;
    min-height: 110px;
    display: grid;
    grid-template-columns: repeat(14, 1fr);
    gap: 4px;
    align-items: end;
  }
  .bcol {
    height: 100%;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    color: var(--faint);
  }
  .bcol i {
    width: 100%;
    border-radius: var(--r-xs) var(--r-xs) 2px 2px;
    background: var(--press);
    transition: background 0.12s, height 0.3s var(--ease);
  }
  .bcol:hover i,
  .bcol.today i {
    background: var(--accent);
  }
  .bcol i.empty {
    background: var(--high);
  }
  .hl {
    margin: 0;
    min-height: 16px;
  }
  .hl b {
    color: var(--text);
  }
  .jump {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .jl {
    font-size: 12px;
    margin-right: 2px;
  }
  .jc {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 180px;
    height: 28px;
    padding: 0 10px;
    border-radius: var(--r-s);
    background: var(--card);
    font-size: 12.5px;
    color: var(--dim);
  }
  .jc:hover {
    background: var(--press);
    color: var(--text);
  }
  .jc img {
    width: 16px;
    height: 16px;
    flex: none;
  }
  .dash {
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(280px, 1fr) minmax(280px, 1fr);
    gap: 12px;
  }
  @media (max-width: 1250px) {
    .dash {
      grid-template-columns: 1fr 1fr;
    }
    .hero {
      grid-column: 1 / -1;
    }
  }
  .card {
    background: var(--panel);
    border-radius: var(--r-l);
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .card h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }
  .card h4 {
    margin: 6px 0 0;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .hello h1 {
    margin: 2px 0 0;
    font-size: 26px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .cap1::first-letter {
    text-transform: uppercase;
  }
  .hello {
    position: relative;
  }
  .clock {
    position: absolute;
    right: 0;
    top: 0;
    font-size: 30px;
    font-weight: 300;
    font-variant-numeric: tabular-nums;
    color: var(--dim);
  }
  .qs {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    padding: 0 4px 0 12px;
    border-radius: var(--r);
    background: var(--high);
    color: var(--faint);
  }
  .qs:focus-within {
    outline: 1px solid var(--line-strong);
  }
  .qs input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: none;
    padding: 0;
    font-size: 14px;
  }
  .qs select {
    height: 30px;
    padding: 0 8px;
    font-size: 12px;
    background: var(--higher);
    border: 0;
    max-width: 160px;
  }
  .acts {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 6px;
  }
  .act {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 40px;
    padding: 0 12px;
    border-radius: var(--r);
    background: var(--high);
    color: var(--dim);
    font-size: 12.5px;
    text-align: left;
    transition: background 0.12s, color 0.12s;
  }
  .act:hover {
    background: var(--higher);
    color: var(--text);
  }
  .act span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ringrow {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .ring {
    width: 128px;
    height: 128px;
    flex: none;
  }
  .ring circle,
  .ring path {
    fill: none;
    stroke-width: 14;
  }
  .ring .track {
    stroke: var(--high);
  }
  .ring .seg {
    transition: opacity 0.15s;
  }
  .ring .seg.dim {
    opacity: 0.25;
  }
  .ring text {
    text-anchor: middle;
    fill: var(--text);
  }
  .ring .big {
    font-size: 22px;
    font-weight: 600;
  }
  .ring .sm {
    font-size: 10px;
    fill: var(--faint);
  }
  .legend {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .legend > b {
    font-size: 18px;
    font-weight: 600;
  }
  .legend > span {
    font-size: 12px;
    margin-bottom: 6px;
  }
  .lg {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 24px;
    padding: 0 6px;
    border-radius: var(--r-xs);
    font-size: 12px;
    text-align: left;
  }
  .lg:hover {
    background: var(--hover);
  }
  .lg .sw {
    width: 9px;
    height: 9px;
    border-radius: 2px;
    flex: none;
  }
  .lg .low {
    color: var(--rec);
  }
  .full {
    width: 100%;
    justify-content: center;
    margin-top: auto;
  }
  .arow {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 28px;
    padding: 0 6px;
    border-radius: var(--r-s);
    font-size: 12.5px;
    text-align: left;
    width: 100%;
  }
  .arow.lnk:hover {
    background: var(--hover);
  }
  .arow.sel {
    background: var(--sel);
  }
  .activity .bar {
    display: block;
    height: 4px;
    margin: -4px 6px 4px;
  }
  .mini {
    width: 20px;
    height: 20px;
    display: flex;
    flex: none;
  }
  .calm {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 400;
  }
  .fbar {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 10px;
  }
  .btn.sm {
    height: 30px;
    padding: 0 12px;
  }
  .home {
    flex: 1;
    overflow-y: auto;
    padding: 8px 24px 40px;
  }
  section {
    margin-top: 22px;
    position: relative;
    transition: opacity 0.15s;
  }
  section.dragging {
    opacity: 0.4;
  }
  section.drop-before::before,
  section.drop-after::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    border-radius: 2px;
    background: var(--accent);
  }
  section.drop-before::before {
    top: -12px;
  }
  section.drop-after::after {
    bottom: -12px;
  }
  .head {
    position: relative;
  }
  .grip {
    position: absolute;
    left: -22px;
    display: flex;
    padding: 4px 2px;
    border-radius: var(--r-xs);
    color: var(--ghost);
    opacity: 0;
    cursor: grab;
    transition: opacity 0.12s;
  }
  .head:hover .grip {
    opacity: 1;
  }
  .grip:hover {
    color: var(--text);
    background: var(--hover);
  }
  .cnt {
    margin-left: 6px;
    font-weight: 500;
    color: var(--ghost);
  }
  .icon-btn.xs {
    width: 26px;
    height: 26px;
  }
  .pad {
    padding: 4px 2px;
  }
  .dir {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: 8px;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px;
    border-radius: var(--r);
    color: var(--dim);
    font-size: 12px;
    text-align: center;
    transition: background 0.12s;
  }
  .tile:hover {
    background: var(--hover);
    color: var(--text);
  }
  .tile.sel {
    background: var(--sel);
    color: var(--text);
  }
  .tt {
    aspect-ratio: 1;
    border-radius: var(--r-s);
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .tile.more .tt {
    font-size: 18px;
    color: var(--text);
  }
  .tn {
    display: block;
  }
  .sys16 {
    width: 16px;
    height: 16px;
    flex: none;
  }
  .fold {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: -4px;
    padding: 4px;
    border-radius: var(--r-s);
    color: inherit;
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
  }
  .fold:hover {
    color: var(--text);
  }
  .chev {
    display: flex;
    transition: transform 0.12s;
  }
  .chev.shut {
    transform: rotate(-90deg);
  }
  h2 {
    margin: 0 0 12px;
    min-height: 30px;
  }
  .btn.sm {
    height: 28px;
    text-transform: none;
    letter-spacing: 0;
    font-weight: 500;
    font-size: 12px;
  }
  .folders {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 8px;
  }
  .folder {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 54px;
    padding: 0 16px;
    border-radius: var(--r);
    background: var(--panel);
    color: var(--dim);
    text-align: left;
    transition: background 0.12s, color 0.12s;
  }
  .folder:hover {
    background: var(--high);
    color: var(--text);
  }
  .sys {
    width: 24px;
    height: 24px;
  }
  .sysbig {
    width: 40px;
    height: 40px;
  }
  .fi {
    display: flex;
    color: var(--text);
  }
  .drives {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 10px;
  }
  .drive {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 16px 18px;
    border-radius: var(--r-l);
    background: var(--panel);
    text-align: left;
    transition: background 0.12s;
  }
  .drive:hover {
    background: var(--high);
  }
  .folder.sel,
  .drive.sel,
  .rrow.sel {
    background: var(--sel);
    color: var(--text);
    outline: 1px solid var(--line-strong);
  }
  .drive.off {
    opacity: 0.5;
  }
  .drive.add {
    background: transparent;
    outline: 1px dashed var(--line-strong);
  }
  .di {
    width: 48px;
    height: 48px;
    border-radius: var(--r-l);
    display: flex;
    align-items: center;
    justify-content: center;
    flex: none;
  }
  .drive .bar {
    display: block;
    margin: 8px 0 6px;
    height: 5px;
  }
  .cap {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }
  .cap .low {
    color: var(--rec);
  }
  .clean {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
    white-space: nowrap;
  }
  .ej {
    display: flex;
    padding: 4px;
    margin-right: 6px;
    border-radius: var(--r-s);
    color: var(--faint);
  }
  .ej:hover {
    background: var(--press);
    color: var(--text);
  }
  .small {
    font-size: 12px;
    display: block;
  }
  b {
    font-weight: 600;
  }
  .recent {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .rrow {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 34px;
    padding-right: 6px;
    border-radius: var(--r);
    color: var(--dim);
  }
  .rrow:hover {
    background: var(--hover);
    color: var(--text);
  }
  .rmain {
    flex: 1;
    min-width: 0;
    height: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px;
    text-align: left;
  }
  .ra {
    width: 26px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-s);
    color: var(--faint);
    opacity: 0;
  }
  .rrow:hover .ra,
  .ra.on {
    opacity: 1;
  }
  .ra.on {
    color: var(--text);
  }
  .ra:hover {
    background: var(--press);
    color: var(--text);
  }
  .recent .path {
    max-width: 55%;
    font-size: 12px;
  }
</style>
