<script lang="ts">
  import Icon from './Icon.svelte'
  import Thumb from './Thumb.svelte'
  import { app, settings, activeTab, transfer, toast, shelfAdd } from '../lib/state.svelte'
  import { size, samePath } from '../lib/util'
  import type { Entry } from '../lib/api'

  const list = $derived(app.shelf)
  const total = $derived(list.reduce((a, e) => a + (e.dir ? 0 : e.size), 0))
  let over = $state(false)
  let picked = $state(new Set<string>())

  const target = $derived(activeTab())
  const canDrop = $derived(!!target && !target.special)
  const chosen = () => (picked.size ? list.filter((e) => picked.has(e.path)) : list)

  async function put(mode: 'copy' | 'move') {
    const t = target
    if (!t || t.special) return toast('Otwórz folder, do którego mają trafić pliki', 'error')
    const items = chosen()
    if (!items.length) return
    await transfer(items.map((e) => e.path), t.path, mode)
    // Przeniesione znikają z półki, skopiowane zostają (można je wkleić jeszcze gdzieś).
    if (mode === 'move') {
      const gone = new Set(items.map((e) => e.path))
      app.shelf = app.shelf.filter((e) => !gone.has(e.path))
    }
    picked = new Set()
  }

  function remove(e: Entry) {
    app.shelf = app.shelf.filter((x) => x.path !== e.path)
    picked.delete(e.path)
    picked = new Set(picked)
  }
  function toggle(e: Entry, ev: MouseEvent) {
    const s = new Set(ev.ctrlKey || ev.shiftKey ? picked : [])
    if (picked.has(e.path) && (ev.ctrlKey || picked.size === 1)) s.delete(e.path)
    else s.add(e.path)
    picked = s
  }

  function dragOver(ev: DragEvent) {
    if (!app.dragging || app.dragging.fromShelf) return
    ev.preventDefault()
    over = true
    ev.dataTransfer!.dropEffect = 'link'
  }
  function drop(ev: DragEvent) {
    ev.preventDefault()
    over = false
    const t = activeTab()
    if (!app.dragging || app.dragging.fromShelf) return
    const paths = app.dragging.paths
    const entries = app.tabs.flatMap((x) => x.items).filter((e) => paths.includes(e.path))
    shelfAdd(entries.length ? entries : t.selectedEntries)
  }
  function dragStart(ev: DragEvent, e: Entry) {
    const items = picked.has(e.path) ? chosen() : [e]
    app.dragging = { paths: items.map((x) => x.path), fromShelf: true }
    ev.dataTransfer!.effectAllowed = 'copyMove'
    ev.dataTransfer!.setData('text/plain', items.map((x) => x.path).join('\n'))
  }
</script>

<section class="shelf fade-in" class:over aria-label="Schowek plików" ondragover={dragOver} ondragleave={() => (over = false)} ondrop={drop}>
  <header>
    <Icon name="archive" size={16} />
    <b class="grow">Schowek plików</b>
    <span class="faint">{list.length ? `${list.length} · ${size(total)}` : ''}</span>
    <button class="icon-btn xs" title="Zamknij (pliki zostaną w schowku)" aria-label="Zamknij schowek plików" onclick={() => (settings.shelfOpen = false)}><Icon name="close" size={14} /></button>
  </header>

  {#if !list.length}
    <div class="empty">
      <Icon name="download" size={22} />
      <p>Przeciągnij tu pliki z różnych folderów,<br />potem przenieś je naraz w jedno miejsce.</p>
      <p class="faint small">Niezależny od schowka Windows (Ctrl+C)</p>
    </div>
  {:else}
    <div class="items" role="listbox" aria-multiselectable="true">
      {#each list as e (e.path)}
        <div class="it" class:sel={picked.has(e.path)} role="option" aria-selected={picked.has(e.path)} tabindex="-1" draggable="true" ondragstart={(ev) => dragStart(ev, e)} ondragend={() => (app.dragging = null)} onclick={(ev) => toggle(e, ev)} onkeydown={() => {}} title={e.path}>
          <span class="th"><Thumb entry={e} px={40} fit="cover" iconOnly={e.dir} /></span>
          <span class="grow nm"><span class="ellipsis">{e.name}</span><span class="faint ellipsis small">{e.path.slice(0, e.path.length - e.name.length - 1)}</span></span>
          <button class="rm" title="Usuń ze schowka" aria-label="Usuń ze schowka" onclick={(ev) => { ev.stopPropagation(); remove(e) }}><Icon name="close" size={12} /></button>
        </div>
      {/each}
    </div>
    <footer>
      <span class="faint small grow ellipsis">{canDrop ? `Do: ${target?.title}` : 'Otwórz folder docelowy'}{picked.size ? ` · zaznaczone: ${picked.size}` : ''}</span>
      <button class="btn sm" disabled={!canDrop} onclick={() => put('copy')} title="Skopiuj do bieżącego folderu">Kopiuj tutaj</button>
      <button class="btn primary sm" disabled={!canDrop || list.some((e) => target && samePath(e.path, target.path))} onclick={() => put('move')} title="Przenieś do bieżącego folderu">Przenieś tutaj</button>
      <button class="icon-btn xs" title="Wyczyść schowek" aria-label="Wyczyść schowek" onclick={() => { app.shelf = []; picked = new Set() }}><Icon name="trash" size={14} /></button>
    </footer>
  {/if}
</section>

<style>
  .shelf {
    position: fixed;
    right: 16px;
    bottom: 40px;
    z-index: 150;
    width: 340px;
    max-height: min(480px, 60vh);
    display: flex;
    flex-direction: column;
    border-radius: var(--r-l);
    background: var(--popup-bg);
    backdrop-filter: var(--popup-blur);
    border: 1px solid var(--line);
    box-shadow: 0 20px 50px var(--shadow);
    overflow: hidden;
  }
  .shelf.over {
    outline: 2px dashed var(--accent);
    outline-offset: -4px;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 8px 8px 14px;
    flex: none;
  }
  header {
    border-bottom: 1px solid var(--line);
  }
  footer {
    border-top: 1px solid var(--line);
    gap: 6px;
  }
  .icon-btn.xs {
    width: 26px;
    height: 26px;
  }
  .btn.sm {
    height: 28px;
    padding: 0 10px;
    font-size: 12px;
  }
  .empty {
    padding: 26px 20px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    text-align: center;
    color: var(--dim);
    font-size: 12.5px;
  }
  .empty p {
    margin: 0;
  }
  .small {
    font-size: 11px;
  }
  .items {
    overflow-y: auto;
    padding: 4px;
  }
  .it {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 6px;
    border-radius: var(--r-s);
  }
  .it:hover {
    background: var(--hover);
  }
  .it.sel {
    background: var(--sel);
  }
  .th {
    width: 36px;
    height: 36px;
    flex: none;
    border-radius: var(--r-xs);
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .nm {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .rm {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-xs);
    color: var(--faint);
    opacity: 0;
  }
  .it:hover .rm {
    opacity: 1;
  }
  .rm:hover {
    background: var(--press);
    color: var(--text);
  }
</style>
