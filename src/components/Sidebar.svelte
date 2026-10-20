<script lang="ts">
  import Icon from './Icon.svelte'
  import type { IconName } from '../lib/icons'
  import {
    app, settings, activeTab, navigate, newTab, transfer, dropMode, addFavorite, removeFavorite, toggleHomeDir, isHomeDir, toast, refreshDrives,
    HOME, SYNC, SETTINGS, runSaved, removeSaved, renameSaved, removeNetPlace, refreshLibraries, prompt, confirm, LIB, type SavedSearch, type NetPlace, type Library, driveShown, showDriveMenu, ejectable, ejectDrive, lowSpace,
  } from '../lib/state.svelte'
  import { api, thumbUrl, type Drive } from '../lib/api'
  import { samePath, size, ago } from '../lib/util'

  const KNOWN_ICON: Record<string, IconName> = {
    home: 'home', desktop: 'desktop', downloads: 'download', documents: 'doc', pictures: 'image', videos: 'video', music: 'music', onedrive: 'cloud',
  }

  let over = $state<string | null>(null)
  const open = (k: string) => !settings.collapsed[k]
  const toggle = (k: string) => (settings.collapsed[k] = !settings.collapsed[k])
  const cur = $derived(activeTab()?.path ?? '')
  const localDrives = $derived(app.drives.filter((d) => d.kind !== 'network' && driveShown(d)))
  const netDrives = $derived(app.drives.filter((d) => d.kind === 'network'))

  function go(path: string, e?: MouseEvent) {
    const t = activeTab()
    if (e && (e.ctrlKey || e.button === 1)) newTab(path, false, t.id)
    else navigate(t, path)
  }

  function dragOver(e: DragEvent, path: string) {
    if (!app.dragging || path.startsWith('ofp:')) return
    e.preventDefault()
    over = path
    e.dataTransfer!.dropEffect = dropMode(app.dragging.paths, path, e) === 'copy' ? 'copy' : 'move'
  }
  function drop(e: DragEvent, path: string) {
    e.preventDefault()
    over = null
    if (app.dragging) transfer(app.dragging.paths, path, dropMode(app.dragging.paths, path, e))
  }
  function favDrop(e: DragEvent) {
    e.preventDefault()
    over = null
    // Upuszczenie folderu na nagłówek „Przypięte" przypina go.
    for (const p of app.dragging?.paths ?? []) api.stat(p).then((s) => s.dir && addFavorite(s.path))
  }

  function driveIcon(d: Drive): IconName {
    if (d.kind === 'removable') return 'usb'
    if (d.kind === 'cdrom') return 'disc'
    if (d.kind === 'network') return 'network'
    return 'drive'
  }
  const used = (d: Drive) => (d.total ? 1 - d.free / d.total : 0)
  const driveName = (d: Drive) => d.label || (!d.online && d.kind === 'removable' ? 'Czytnik – brak nośnika' : d.kind === 'removable' ? 'Dysk wymienny' : d.kind === 'network' ? (d.remote ?? 'Dysk sieciowy') : 'Dysk lokalny')

  const driveMenu = (e: MouseEvent, d: Drive) => showDriveMenu(e, d, (p) => go(p), (p) => newTab(p, false, activeTab().id))

  function folderMenu(e: MouseEvent, path: string, fav: boolean) {
    e.preventDefault()
    app.menu = {
      x: e.clientX, y: e.clientY,
      items: [
        { label: 'Otwórz w nowej karcie', icon: 'plus', run: () => newTab(path) },
        { label: 'Pokaż w Eksploratorze', icon: 'open', run: () => api.reveal(path) },
        fav ? { label: 'Odepnij', icon: 'pin', run: () => removeFavorite(path) } : { label: 'Przypnij', icon: 'pin', run: () => addFavorite(path) },
        { label: isHomeDir(path) ? 'Usuń z ekranu Komputer' : 'Pokaż na ekranie Komputer', icon: 'grid', run: () => toggleHomeDir(path) },
      ],
    }
  }

  function placeMenu(e: MouseEvent, p: NetPlace) {
    e.preventDefault()
    app.menu = {
      x: e.clientX, y: e.clientY,
      items: [
        { label: 'Otwórz', icon: 'folder', run: () => go(p.path) },
        { label: 'Otwórz w nowej karcie', icon: 'plus', run: () => newTab(p.path) },
        { label: 'Kopiuj adres', icon: 'link', run: () => { navigator.clipboard.writeText('ftp://' + p.path.slice(6).replace(/\\/g, '/')); toast('Skopiowano adres') } },
        { sep: true },
        { label: 'Usuń z listy', icon: 'unlink', run: () => removeNetPlace(p, false) },
        { label: 'Usuń i zapomnij hasło', icon: 'unlink', danger: true, run: () => removeNetPlace(p, true) },
      ],
    }
  }

  // ---------- Biblioteki Windows ----------
  const libPath = (l: { path: string }) => `${LIB}|${l.path}`
  async function newLibrary() {
    const name = await prompt('Nowa biblioteka', 'Nowa biblioteka', 'Dalej', false, 'Biblioteka zbiera kilka folderów w jednym miejscu – będzie widoczna także w Eksploratorze Windows.')
    if (!name) return
    const folder = await api.pickFolder()
    try {
      const path = await api.libraryCreate(name, folder ?? undefined)
      await refreshLibraries()
      go(`${LIB}|${path}`)
      toast(`Utworzono bibliotekę „${name}”`, 'ok')
    } catch (e) {
      toast(String(e), 'error')
    }
  }
  async function libAdd(l: Library, folder?: string) {
    const f = folder ?? (await api.pickFolder())
    if (!f) return
    try {
      await api.libraryAdd(l.path, f)
      await refreshLibraries()
      toast(`Dodano do biblioteki „${l.name}”`, 'ok')
    } catch (e) {
      toast(String(e), 'error')
    }
  }
  function libDrop(e: DragEvent, l: Library) {
    e.preventDefault()
    over = null
    const dirs = app.dragging?.paths.filter((p) => app.tabs.some((t) => t.items.some((x) => x.dir && x.path === p))) ?? []
    if (!dirs.length) return toast('Do biblioteki można dodawać tylko foldery')
    dirs.reduce((acc, d) => acc.then(() => libAdd(l, d)), Promise.resolve())
  }
  function libMenu(e: MouseEvent, l: Library) {
    e.preventDefault()
    app.menu = {
      x: e.clientX, y: e.clientY,
      items: [
        { label: 'Otwórz', icon: 'folder', run: () => go(libPath(l)) },
        { label: 'Otwórz w nowej karcie', icon: 'plus', run: () => newTab(libPath(l)) },
        { sep: true },
        { label: 'Dodaj folder…', icon: 'folderPlus', run: () => libAdd(l) },
        ...(l.folders.length
          ? [{ label: 'Usuń folder z biblioteki', icon: 'unlink', sub: l.folders.map((f) => ({ label: f, run: () => api.libraryRemove(l.path, f).then(refreshLibraries).catch((er) => toast(String(er), 'error')) })) }]
          : []),
        { sep: true },
        { label: 'Usuń bibliotekę', icon: 'trash', danger: true, run: async () => {
          if (!(await confirm('Usunąć bibliotekę?', `„${l.name}” zniknie z OpenFilePower i z Eksploratora. Foldery i pliki zostaną nietknięte.`, 'Usuń', true))) return
          await api.startOp('trash', [l.path])
          setTimeout(refreshLibraries, 400)
        } },
      ],
    }
  }

  function savedMenu(e: MouseEvent, s: SavedSearch) {
    e.preventDefault()
    app.menu = {
      x: e.clientX, y: e.clientY,
      items: [
        { label: 'Uruchom', icon: 'search', run: () => runSaved(activeTab(), s) },
        { label: 'Uruchom w nowej karcie', icon: 'plus', run: () => runSaved(newTab(s.root), s) },
        { sep: true },
        { label: 'Zmień nazwę', icon: 'edit', run: () => renameSaved(s) },
        { label: 'Usuń', icon: 'trash', danger: true, run: () => removeSaved(s.id) },
      ],
    }
  }
  const activeSearch = $derived(activeTab()?.search?.query ?? null)

  function syncDot(id: string) {
    const s = app.syncStatus[id]
    if (!s) return ''
    if (s.state === 'syncing' || s.state === 'scanning') return 'busy'
    if (s.state === 'error' || s.state === 'confirm') return 'rec'
    if (s.state === 'paused') return ''
    return s.connected ? 'ok' : ''
  }

  let resizing = false
  function startResize(e: PointerEvent) {
    resizing = true
    ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
  }
  function onResize(e: PointerEvent) {
    if (resizing) settings.sidebarWidth = Math.max(190, Math.min(420, e.clientX))
  }
</script>

<nav class="sidebar" style:width="{settings.sidebarWidth}px" aria-label="Panel boczny" data-region="sidebar">
  <div class="scroll">
    <button class="item" class:on={cur === HOME} onclick={() => go(HOME)}>
      <Icon name="pc" /><span class="ellipsis grow">Komputer</span>
    </button>

    {#if settings.showQuickAccess}
    <button class="sec" onclick={() => toggle('quick')}><span class="chev" class:shut={!open('quick')}><Icon name="chevronDown" size={13} /></span><span class="grow">Szybki dostęp</span></button>
    {#if open('quick')}
    {#each app.known as k (k.id)}
      <button
        class="item"
        class:on={samePath(cur, k.path)}
        class:over={over === k.path}
        onclick={(e) => go(k.path, e)}
        onauxclick={(e) => go(k.path, e)}
        oncontextmenu={(e) => folderMenu(e, k.path, false)}
        ondragover={(e) => dragOver(e, k.path)}
        ondragleave={() => (over = null)}
        ondrop={(e) => drop(e, k.path)}
      >
        <img class="sys" src={thumbUrl(k.path, 18, 'i')} alt="" /><span class="ellipsis grow">{k.name}</span>
      </button>
    {/each}
    {/if}
    {/if}

    {#if settings.sidebar.showSaved}
    {#if settings.savedSearches.length}
      <button class="sec" onclick={() => toggle('saved')}><span class="chev" class:shut={!open('saved')}><Icon name="chevronDown" size={13} /></span><span class="grow">Zapisane wyszukiwania</span><span class="cnt">{settings.savedSearches.length}</span></button>
      {#if open('saved')}
        {#each settings.savedSearches as s (s.id)}
          <button class="item" class:on={activeSearch === s.query && samePath(cur, s.root)} onclick={(e) => (e.ctrlKey ? runSaved(newTab(s.root), s) : runSaved(activeTab(), s))} oncontextmenu={(e) => savedMenu(e, s)} title={`${s.query}\nw ${s.root}`}>
            <Icon name="search" size={16} /><span class="ellipsis grow">{s.name}</span>
          </button>
        {/each}
      {/if}
    {/if}
    {/if}

    {#if settings.sidebar.showFav}
    <div
      class="sec"
      class:over={over === '#fav'}
      role="group"
      ondragover={(e) => { if (app.dragging) { e.preventDefault(); over = '#fav' } }}
      ondragleave={() => (over = null)}
      ondrop={favDrop}
    ><button class="sec-btn" onclick={() => toggle('fav')}><span class="chev" class:shut={!open('fav')}><Icon name="chevronDown" size={13} /></span><span class="grow">Przypięte</span>{#if app.favorites.length}<span class="cnt">{app.favorites.length}</span>{/if}</button></div>
    {#if open('fav')}
    {#each app.favorites as f (f.path)}
      <button
        class="item"
        class:on={samePath(cur, f.path)}
        class:over={over === f.path}
        onclick={(e) => go(f.path, e)}
        onauxclick={(e) => go(f.path, e)}
        oncontextmenu={(e) => folderMenu(e, f.path, true)}
        ondragover={(e) => dragOver(e, f.path)}
        ondragleave={() => (over = null)}
        ondrop={(e) => drop(e, f.path)}
        title={f.path}
      >
        <img class="sys" src={thumbUrl(f.path, 18, 'i')} alt="" /><span class="ellipsis grow">{f.name}</span>
      </button>
    {:else}
      <div class="hint">Przeciągnij tu folder albo wybierz „Przypnij” z menu</div>
    {/each}
    {/if}
    {/if}

    {#if settings.sidebar.showLibs}
    <div class="sec row"><button class="sec-btn" onclick={() => toggle('libs')}><span class="chev" class:shut={!open('libs')}><Icon name="chevronDown" size={13} /></span><span class="grow">Biblioteki</span></button>
      <button class="mini" title="Nowa biblioteka" aria-label="Nowa biblioteka" onclick={newLibrary}><Icon name="plus" size={14} /></button>
    </div>
    {#if open('libs')}
    {#each app.libraries as l (l.path)}
      <button
        class="item"
        class:on={cur === libPath(l)}
        class:over={over === l.path}
        onclick={(e) => go(libPath(l), e)}
        onauxclick={(e) => go(libPath(l), e)}
        oncontextmenu={(e) => libMenu(e, l)}
        ondragover={(e) => { if (app.dragging) { e.preventDefault(); over = l.path } }}
        ondragleave={() => (over = null)}
        ondrop={(e) => libDrop(e, l)}
        title={l.folders.join('\n') || 'Biblioteka bez folderów – przeciągnij tu folder'}
      >
        <img class="sys" src={thumbUrl(l.path, 18, 'i')} alt="" /><span class="ellipsis grow">{l.name}</span>{#if l.folders.length > 1}<span class="cnt">{l.folders.length}</span>{/if}
      </button>
    {:else}
      <button class="hint link" onclick={newLibrary}>Utwórz bibliotekę…</button>
    {/each}
    {/if}
    {/if}

    {#if settings.sidebar.showDrives}
    <button class="sec" onclick={() => toggle('drives')}><span class="chev" class:shut={!open('drives')}><Icon name="chevronDown" size={13} /></span><span class="grow">Dyski</span><span class="cnt">{localDrives.length}</span></button>
    {#if open('drives')}
    {#each localDrives as d (d.path)}
      <button
        class="item drive"
        class:on={samePath(cur, d.path)}
        class:off={!d.online}
        class:over={over === d.path}
        onclick={(e) => d.online ? go(d.path, e) : toast(`${d.letter} – brak nośnika`)}
        onauxclick={(e) => go(d.path, e)}
        oncontextmenu={(e) => driveMenu(e, d)}
        ondragover={(e) => dragOver(e, d.path)}
        ondragleave={() => (over = null)}
        ondrop={(e) => drop(e, d.path)}
        title="{driveName(d)} ({d.letter}) · {size(d.free)} wolne z {size(d.total)}"
      >
        <img class="sys" src={thumbUrl(d.path, 18, 'i')} alt="" />
        <span class="grow">
          <span class="row"><span class="ellipsis grow">{driveName(d)}</span>{#if ejectable(d)}<span class="ej" role="button" tabindex="-1" title="Wysuń bezpiecznie" onclick={(e) => { e.stopPropagation(); ejectDrive(d) }} onkeydown={() => {}}><Icon name="eject" size={13} /></span>{/if}<span class="faint letter">{d.letter}</span></span>
          {#if d.online && d.total}
            <span class="bar" class:warn={used(d) > 0.8} class:full={used(d) > 0.9}><i style:width="{used(d) * 100}%"></i></span>
            <span class="sub cap" class:low={lowSpace(d)}><span>{size(d.free)} wolne</span><span>z {size(d.total)}</span></span>
          {:else if !d.online}
            <span class="sub">brak nośnika</span>
          {/if}
        </span>
      </button>
    {/each}
    {/if}
    {/if}

    {#if settings.sidebar.showNet}
    <div class="sec row"><button class="sec-btn" onclick={() => toggle('net')}><span class="chev" class:shut={!open('net')}><Icon name="chevronDown" size={13} /></span><span class="grow">Sieć</span></button>
      <button class="mini" title="Połącz z lokalizacją sieciową" aria-label="Połącz z lokalizacją sieciową" onclick={() => (app.dialog = { type: 'network' })}><Icon name="plus" size={14} /></button>
    </div>
    {#if open('net')}
    {#each netDrives as d (d.letter)}
      <button
        class="item drive"
        class:on={samePath(cur, d.path)}
        class:off={!d.online}
        class:over={over === d.path}
        onclick={(e) => d.online ? go(d.path, e) : toast(`${d.remote} jest teraz niedostępny`)}
        oncontextmenu={(e) => driveMenu(e, d)}
        ondragover={(e) => d.online && dragOver(e, d.path)}
        ondragleave={() => (over = null)}
        ondrop={(e) => drop(e, d.path)}
        title="{d.remote ?? ''} {d.online ? '' : '· rozłączony'}"
      >
        <img class="sys" src={thumbUrl(d.path, 18, 'i')} alt="" />
        <span class="grow">
          <span class="row"><span class="ellipsis grow">{d.label || d.remote?.split('\\').filter(Boolean).pop() || 'Udział'}</span><span class="faint letter">{d.letter}</span></span>
          <span class="sub ellipsis">{d.online ? d.remote : 'rozłączony'}</span>
        </span>
      </button>
    {/each}
    {#each settings.netPlaces as p (p.id)}
      <button class="item" class:on={samePath(cur, p.path)} class:over={over === p.path} onclick={(e) => go(p.path, e)} onauxclick={(e) => go(p.path, e)} oncontextmenu={(e) => placeMenu(e, p)} ondragover={(e) => dragOver(e, p.path)} ondragleave={() => (over = null)} ondrop={(e) => drop(e, p.path)} title={'ftp://' + p.path.slice(6).replace(/\\/g, '/')}>
        <Icon name="network" size={17} /><span class="grow"><span class="ellipsis" style="display:block">{p.name}</span><span class="sub ellipsis">FTP</span></span>
      </button>
    {/each}
    {#if !netDrives.length && !settings.netPlaces.length}
      <button class="hint link" onclick={() => (app.dialog = { type: 'network' })}>Połącz z lokalizacją sieciową…</button>
    {/if}
    {/if}
    {/if}

    {#if settings.sidebar.showSync}
    <div class="sec row"><button class="sec-btn" onclick={() => toggle('sync')}><span class="chev" class:shut={!open('sync')}><Icon name="chevronDown" size={13} /></span><span class="grow">Synchronizacja</span></button>
      <button class="mini" title="Nowa synchronizacja" onclick={() => (app.dialog = { type: 'sync', profile: null, source: activeTab()?.special ? undefined : activeTab()?.path })}><Icon name="plus" size={14} /></button>
    </div>
    {#if open('sync')}
    {#each app.syncProfiles as p (p.id)}
      {@const s = app.syncStatus[p.id]}
      <button class="item" class:on={cur === SYNC} onclick={() => go(SYNC)} title={s?.last_result ?? ''}>
        <span class="dot {syncDot(p.id)}"></span>
        <span class="grow">
          <span class="ellipsis" style="display:block">{p.name}</span>
          <span class="sub ellipsis">
            {#if s?.state === 'syncing'}synchronizuję {Math.round((s.progress ?? 0) * 100)}%
            {:else if s?.state === 'confirm'}czeka na potwierdzenie
            {:else if s?.state === 'paused'}wstrzymane
            {:else if s?.connected}aktualne · {ago(s.last_sync)}
            {:else}cel odłączony · był {ago(s?.last_seen ?? 0)}{/if}
          </span>
        </span>
      </button>
    {:else}
      <button class="hint link" onclick={() => go(SYNC)}>Synchronizuj folder z dyskiem…</button>
    {/each}
    {/if}
    {/if}
  </div>

  <div class="foot">
    <button class="item" class:on={cur === SETTINGS} onclick={() => go(SETTINGS)}><Icon name="settings" /><span class="grow">Ustawienia</span></button>
  </div>
  <div class="resize" role="separator" onpointerdown={startResize} onpointermove={onResize} onpointerup={() => (resizing = false)}></div>
</nav>

<style>
  .sidebar {
    position: relative;
    flex: none;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border-radius: 0 var(--r-l) 0 0;
    min-height: 0;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 10px 8px 12px;
  }
  .sec {
    font-family: inherit;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--faint);
    padding: 14px 8px 6px;
    border-radius: var(--r-s);
  }
  button.sec,
  .sec-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    text-align: left;
    font-family: inherit;
    color: inherit;
  }
  .sec-btn {
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
    padding: 0;
    flex: 1;
  }
  button.sec:hover,
  .sec-btn:hover {
    color: var(--dim);
  }
  .chev {
    display: flex;
    transition: transform 0.15s var(--ease);
  }
  .chev.shut {
    transform: rotate(-90deg);
  }
  .cnt {
    font-weight: 500;
    color: var(--ghost);
  }
  .sec.over {
    background: var(--sel);
    color: var(--text);
  }
  .item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 11px;
    min-height: 34px;
    padding: 6px 10px;
    border-radius: var(--r);
    color: var(--dim);
    text-align: left;
    transition: background 0.1s, color 0.1s;
  }
  .item:hover {
    background: var(--hover);
    color: var(--text);
  }
  .item.on {
    background: var(--sel);
    color: var(--text);
  }
  .item.over {
    background: var(--sel-strong);
    color: var(--text);
    outline: 1px solid var(--line-strong);
  }
  .item.off {
    opacity: 0.5;
  }
  .sys {
    width: 18px;
    height: 18px;
    flex: none;
  }
  .drive .bar {
    display: block;
    margin-top: 5px;
    height: 3px;
  }
  .cap {
    display: flex;
    justify-content: space-between;
    gap: 6px;
    margin-top: 3px;
    font-variant-numeric: tabular-nums;
  }
  .cap.low span:first-child {
    color: var(--rec);
  }
  .ej {
    display: flex;
    padding: 2px;
    margin-right: 4px;
    border-radius: var(--r-xs);
    color: var(--faint);
    opacity: 0;
    transition: opacity 0.12s;
  }
  .drive:hover .ej {
    opacity: 1;
  }
  .ej:hover {
    background: var(--press);
    color: var(--text);
  }
  .letter {
    font-size: 11px;
  }
  .sub {
    display: block;
    font-size: 11px;
    color: var(--faint);
  }
  .hint {
    display: block;
    padding: 4px 10px 6px;
    font-size: 12px;
    color: var(--ghost);
    text-align: left;
  }
  .hint.link:hover {
    color: var(--dim);
  }
  .mini {
    width: 20px;
    height: 20px;
    border-radius: var(--r-xs);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--faint);
  }
  .mini:hover {
    background: var(--press);
    color: var(--text);
  }
  .foot {
    padding: 6px 8px 10px;
  }
  .resize {
    position: absolute;
    top: 0;
    right: -3px;
    width: 6px;
    height: 100%;
    cursor: col-resize;
    z-index: 2;
  }
</style>
