<script lang="ts">
  import Icon from './Icon.svelte'
  import type { IconName } from '../lib/icons'
  import { app, settings, navigate, newTab, addFavorite, removeFavorite, refreshDrives, toast, driveShown, showDriveMenu, ejectable, ejectDrive, lowSpace, SYNC, type Tab, type MenuItem } from '../lib/state.svelte'
  import { size, ago, basename } from '../lib/util'
  import { api, thumbUrl, type Drive } from '../lib/api'

  let { tab }: { tab: Tab } = $props()

  const KNOWN_ICON: Record<string, IconName> = {
    home: 'home', desktop: 'desktop', downloads: 'download', documents: 'doc', pictures: 'image', videos: 'video', music: 'music', onedrive: 'cloud',
  }
  const used = (d: Drive) => (d.total ? 1 - d.free / d.total : 0)
  const go = (p: string, e?: MouseEvent) => (e?.ctrlKey || e?.button === 1 ? newTab(p, false, tab.id) : navigate(tab, p))
  const recent = $derived(app.recent.slice(0, 10))

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

  const groups = $derived([
    { title: 'Dyski', list: app.drives.filter((d) => d.kind === 'fixed' || d.kind === 'ram' || d.kind === 'unknown') },
    { title: 'Urządzenia wymienne', list: app.drives.filter((d) => (d.kind === 'removable' || d.kind === 'cdrom') && driveShown(d)) },
    { title: 'Lokalizacje sieciowe', list: app.drives.filter((d) => d.kind === 'network') },
  ])
</script>

<div class="home" role="presentation" onmousedown={(e) => { if (!(e.target as HTMLElement).closest('button')) sel = null }}>
  <section>
    <h2 class="section-title">Foldery</h2>
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
  </section>

  {#each groups as g (g.title)}
    {#if g.list.length || g.title === 'Lokalizacje sieciowe'}
      <section>
        <h2 class="section-title row">
          <span class="grow">{g.title}</span>
          {#if g.title === 'Lokalizacje sieciowe'}
            <button class="btn ghost sm" onclick={() => (app.dialog = { type: 'network' })}><Icon name="plus" size={15} />Podłącz dysk sieciowy</button>
          {/if}
        </h2>
        <div class="drives">
          {#each g.list as d (d.letter)}
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
          {:else}
            <button class="drive add" onclick={() => (app.dialog = { type: 'network' })}>
              <span class="di"><Icon name="plus" size={22} /></span>
              <span class="grow"><b>Podłącz udział SMB / NAS</b><span class="faint small" style="display:block">np. \\nas\zdjecia – logowanie w oknie Windows</span></span>
            </button>
          {/each}
        </div>
      </section>
    {/if}
  {/each}

  {#if app.syncProfiles.length}
    <section>
      <h2 class="section-title">Synchronizacja</h2>
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
    </section>
  {/if}

  {#if recent.length}
    <section>
      <h2 class="section-title">Ostatnio otwierane</h2>
      <div class="recent">
        {#each recent as r (r)}
          <button class:sel={sel === r} onclick={(e) => click(e, r)} ondblclick={() => dbl(r)} onkeydown={(e) => key(e, r)} onauxclick={(e) => aux(e, r)} oncontextmenu={(e) => folderMenu(e, r)} title={r}><Icon name="folder" size={16} /><span class="ellipsis grow">{basename(r)}</span><span class="faint ellipsis path">{r}</span></button>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .home {
    flex: 1;
    overflow-y: auto;
    padding: 8px 24px 40px;
  }
  section {
    margin-top: 22px;
    max-width: 1300px;
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
  .recent button.sel {
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
    border-radius: 14px;
    background: var(--high);
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
    border-radius: 7px;
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
  .recent button {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 34px;
    padding: 0 12px;
    border-radius: 10px;
    color: var(--dim);
    text-align: left;
  }
  .recent button:hover {
    background: var(--hover);
    color: var(--text);
  }
  .recent .path {
    max-width: 55%;
    font-size: 12px;
  }
</style>
