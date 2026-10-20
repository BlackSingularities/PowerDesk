<script lang="ts">
  import { app, settings, type Tab } from '../lib/state.svelte'
  import { size, items, volumeOf } from '../lib/util'

  let { tab }: { tab: Tab } = $props()
  const sel = $derived(tab.selectedEntries)
  const selSize = $derived(sel.reduce((a, e) => a + (e.dir ? 0 : e.size), 0))
  const drive = $derived(app.drives.find((d) => !tab.special && volumeOf(d.path) === volumeOf(tab.path)))
  const hiddenCount = $derived(settings.showHidden ? 0 : tab.source.filter((e) => e.hidden || e.system).length)
  const syncing = $derived(Object.values(app.syncStatus).filter((s) => s.state === 'syncing' || s.state === 'scanning').length)
</script>

<footer class="status">
  {#if !tab.special}
    <span>{items(tab.items.length)}</span>
    {#if sel.length}<span class="sep">·</span><span>zaznaczono {sel.length}{selSize ? ` (${size(selSize)})` : ''}</span>{/if}
    {#if hiddenCount}<span class="sep">·</span><button class="faint" onclick={() => (settings.showHidden = true)} title="Pokaż ukryte (Ctrl+H)">{hiddenCount} ukrytych</button>{/if}
    {#if tab.search}<span class="sep">·</span><span>{tab.search.running ? 'szukam…' : 'zakończono'} · przejrzano {tab.search.scanned.toLocaleString('pl-PL')}</span>{/if}
  {/if}
  <span class="grow"></span>
  {#if syncing}<span class="row"><span class="dot busy"></span>synchronizacja w toku</span><span class="sep">·</span>{/if}
  {#if drive?.total}<span>{drive.letter} · {size(drive.free)} wolne</span>{/if}
</footer>

<style>
  .status {
    height: 26px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 16px;
    font-size: 12px;
    color: var(--faint);
  }
  .sep {
    color: var(--ghost);
  }
  button:hover {
    color: var(--text);
  }
</style>
