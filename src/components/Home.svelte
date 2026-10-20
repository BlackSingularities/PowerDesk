<script lang="ts">
  import Icon from './Icon.svelte'
  import type { IconName } from '../lib/icons'
  import { app, navigate, newTab, SYNC, type Tab } from '../lib/state.svelte'
  import { size, ago, basename } from '../lib/util'
  import type { Drive } from '../lib/api'

  let { tab }: { tab: Tab } = $props()

  const KNOWN_ICON: Record<string, IconName> = {
    home: 'home', desktop: 'desktop', downloads: 'download', documents: 'doc', pictures: 'image', videos: 'video', music: 'music', onedrive: 'cloud',
  }
  const used = (d: Drive) => (d.total ? 1 - d.free / d.total : 0)
  const go = (p: string, e?: MouseEvent) => (e?.ctrlKey || e?.button === 1 ? newTab(p, false, tab.id) : navigate(tab, p))
  const recent = $derived(app.recent.slice(0, 10))
  const groups = $derived([
    { title: 'Dyski', list: app.drives.filter((d) => d.kind === 'fixed' || d.kind === 'ram' || d.kind === 'unknown') },
    { title: 'Urządzenia wymienne', list: app.drives.filter((d) => d.kind === 'removable' || d.kind === 'cdrom') },
    { title: 'Lokalizacje sieciowe', list: app.drives.filter((d) => d.kind === 'network') },
  ])
</script>

<div class="home">
  <section>
    <h2 class="section-title">Foldery</h2>
    <div class="folders">
      {#each app.known as k (k.id)}
        <button class="folder" onclick={(e) => go(k.path, e)} onauxclick={(e) => go(k.path, e)}>
          <span class="fi"><Icon name={KNOWN_ICON[k.id] ?? 'folder'} size={22} /></span>
          <span class="ellipsis">{k.name}</span>
        </button>
      {/each}
      {#each app.favorites as f (f.path)}
        <button class="folder" onclick={(e) => go(f.path, e)} title={f.path}>
          <span class="fi"><Icon name="pin" size={20} /></span>
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
            <button class="drive" class:off={!d.online} onclick={(e) => d.online && go(d.path, e)} title={d.remote ?? d.path}>
              <span class="di"><Icon name={d.kind === 'network' ? 'network' : d.kind === 'removable' ? 'usb' : d.kind === 'cdrom' ? 'disc' : 'drive'} size={26} /></span>
              <span class="grow">
                <span class="row"><b class="ellipsis grow">{d.label || (d.kind === 'network' ? d.remote?.split('\\').filter(Boolean).pop() : 'Dysk lokalny')}</b><span class="faint">{d.letter}</span></span>
                {#if d.online && d.total}
                  <span class="bar" class:warn={used(d) > 0.85} class:full={used(d) > 0.95}><i style:width="{used(d) * 100}%"></i></span>
                  <span class="faint small">{size(d.free)} wolne z {size(d.total)} · {d.fs}</span>
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
          <button onclick={(e) => go(r, e)} title={r}><Icon name="folder" size={16} /><span class="ellipsis grow">{basename(r)}</span><span class="faint ellipsis path">{r}</span></button>
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
