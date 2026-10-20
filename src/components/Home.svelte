<script lang="ts">
  import Icon from './Icon.svelte'
  import Thumb from './Thumb.svelte'
  import {
    app, settings, navigate, newTab, addFavorite, removeFavorite, refreshDrives, toast, driveShown, showDriveMenu, ejectable, ejectDrive, lowSpace,
    toggleHomeDir, isHomeDir, removeRecent, toggleRecentPin, clearRecent, removeNetPlace, SYNC, type Tab,
  } from '../lib/state.svelte'
  import { size, ago, basename, samePath, collator, isMedia } from '../lib/util'
  import { api, thumbUrl, type Drive, type Entry } from '../lib/api'

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

  // ---------- Sekcje: kolejność i przeciąganie ----------

  const BASE = ['folders', 'drives', 'removable', 'network', 'sync', 'recent']
  const TITLES: Record<string, string> = {
    folders: 'Foldery', drives: 'Dyski', removable: 'Urządzenia wymienne', network: 'Lokalizacje sieciowe', sync: 'Synchronizacja', recent: 'Ostatnio otwierane',
  }
  const order = $derived.by(() => {
    const want = [...BASE, ...settings.homeDirs.map((p) => 'dir:' + p)]
    const kept = settings.homeOrder.filter((id) => want.includes(id))
    // Nowe sekcje: bazowe na koniec, foldery z zawartością przed „Ostatnio otwierane”.
    for (const id of want) {
      if (kept.includes(id)) continue
      const at = id.startsWith('dir:') ? kept.indexOf('recent') : -1
      if (at >= 0) kept.splice(at, 0, id)
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
      app.viewer = { list: media, index: Math.max(0, media.findIndex((x) => x.path === e.path)) }
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
          {#if id === 'folders'}{@render folders()}
          {:else if id === 'drives' || id === 'removable' || id === 'network'}{@render drives(id)}
          {:else if id === 'sync'}{@render syncs()}
          {:else if id === 'recent'}{@render recents()}
          {:else}{@render dirView(id.slice(4))}{/if}
        {/if}
      </section>
    {/if}
  {/each}
</div>

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
