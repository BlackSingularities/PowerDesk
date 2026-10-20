<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import Icon from './Icon.svelte'
  import { app, activate, closeTab, newTab, transfer, dropMode, togglePin, updateNow, HOME, SYNC, SETTINGS, type Tab, activeTab } from '../lib/state.svelte'
  import type { IconName } from '../lib/icons'

  const win = getCurrentWindow()
  let maximized = $state(false)
  win.isMaximized().then((m) => (maximized = m))
  win.onResized(async () => (maximized = await win.isMaximized()))

  let dragTab: string | null = null
  let overTab = $state<string | null>(null)
  let hoverTimer: ReturnType<typeof setTimeout> | undefined

  function iconFor(t: Tab): IconName {
    if (t.search) return 'search'
    if (t.path === HOME) return 'pc'
    if (t.path === SYNC) return 'sync'
    if (t.path === SETTINGS) return 'settings'
    if (/^[a-zA-Z]:\\?$/.test(t.path)) return 'drive'
    if (t.path.startsWith('\\\\')) return 'network'
    return 'folder'
  }

  function onTabDragStart(e: DragEvent, t: Tab) {
    dragTab = t.id
    e.dataTransfer!.effectAllowed = 'move'
    e.dataTransfer!.setData('text/ofp-tab', t.id)
  }

  function onTabDragOver(e: DragEvent, t: Tab) {
    e.preventDefault()
    if (overTab !== t.id) {
      overTab = t.id
      clearTimeout(hoverTimer)
      // Przytrzymanie plików nad kartą przełącza na nią – jak w przeglądarce.
      if (app.dragging) hoverTimer = setTimeout(() => activate(t.id), 650)
    }
    if (e.dataTransfer) e.dataTransfer.dropEffect = app.dragging ? (dropMode(app.dragging.paths, t.path, e) === 'copy' ? 'copy' : 'move') : 'move'
  }

  function onTabDrop(e: DragEvent, t: Tab) {
    e.preventDefault()
    clearTimeout(hoverTimer)
    overTab = null
    if (dragTab) {
      const from = app.tabs.findIndex((x) => x.id === dragTab)
      const to = app.tabs.findIndex((x) => x.id === t.id)
      if (from >= 0 && to >= 0 && from !== to) {
        const [m] = app.tabs.splice(from, 1)
        // Karta upuszczona między przypiętymi sama się przypina (i odwrotnie).
        m.pinned = t.pinned
        app.tabs.splice(to, 0, m)
      }
      dragTab = null
      return
    }
    if (app.dragging && !t.special) transfer(app.dragging.paths, t.path, dropMode(app.dragging.paths, t.path, e))
  }

  function tabMenu(e: MouseEvent, t: Tab) {
    e.preventDefault()
    const others = app.tabs.filter((x) => x.id !== t.id && !x.pinned)
    const right = app.tabs.slice(app.tabs.indexOf(t) + 1).filter((x) => !x.pinned)
    app.menu = {
      x: e.clientX, y: e.clientY,
      items: [
        { label: t.pinned ? 'Odepnij kartę' : 'Przypnij kartę', icon: 'pin', run: () => togglePin(t) },
        { label: 'Duplikuj kartę', icon: 'copy', run: () => newTab(t.path, true, t.id) },
        { sep: true },
        { label: 'Zamknij kartę', icon: 'close', kbd: 'Ctrl+W', disabled: t.pinned, run: () => closeTab(t.id) },
        { label: 'Zamknij pozostałe karty', disabled: !others.length, run: () => others.forEach((x) => closeTab(x.id)) },
        { label: 'Zamknij karty po prawej', disabled: !right.length, run: () => right.forEach((x) => closeTab(x.id)) },
      ],
    }
  }

  function paneOf(id: string) {
    return app.panes.indexOf(id)
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  <button class="icon-btn side" title="Panel boczny (Ctrl+B)" onclick={() => (app.sidebarOpen = !app.sidebarOpen)}>
    <Icon name="sidebar" />
  </button>

  <div class="tabs" role="tablist" tabindex="-1" data-tauri-drag-region ondblclick={(e) => e.target === e.currentTarget && newTab(activeTab()?.path ?? HOME)}>
    {#each app.tabs as t (t.id)}
      {@const p = paneOf(t.id)}
      <div
        class="tab"
        class:active={p === app.focusedPane}
        class:visible={p >= 0}
        class:over={overTab === t.id}
        class:pinned={t.pinned}
        role="tab"
        tabindex="0"
        aria-selected={p === app.focusedPane}
        draggable="true"
        title={(t.pinned ? '📌 ' : '') + (t.path.startsWith('ofp:') ? t.title : t.path)}
        onclick={() => activate(t.id)}
        onkeydown={(e) => e.key === 'Enter' && activate(t.id)}
        onauxclick={(e) => e.button === 1 && closeTab(t.id)}
        oncontextmenu={(e) => tabMenu(e, t)}
        ondragstart={(e) => onTabDragStart(e, t)}
        ondragover={(e) => onTabDragOver(e, t)}
        ondragleave={() => {
          if (overTab === t.id) overTab = null
          clearTimeout(hoverTimer)
        }}
        ondrop={(e) => onTabDrop(e, t)}
        ondragend={() => (dragTab = null)}
      >
        <span class="ti"><Icon name={iconFor(t)} size={15} /></span>
        {#if !t.pinned}<span class="ellipsis grow">{t.title}</span>{/if}
        {#if t.loading || t.search?.running}<span class="dot busy"></span>{/if}
        {#if app.split && p >= 0}<span class="pane-tag">{p === 0 ? 'L' : 'P'}</span>{/if}
        {#if !t.pinned}
          <button class="x" title="Zamknij kartę (Ctrl+W)" onclick={(e) => { e.stopPropagation(); closeTab(t.id) }}>
            <Icon name="close" size={13} />
          </button>
        {/if}
      </div>
    {/each}
    <button class="icon-btn add" title="Nowa karta (Ctrl+T)" onclick={() => newTab(activeTab()?.path ?? HOME, true)}>
      <Icon name="plus" size={16} />
    </button>
  </div>

  {#if app.update}
    <button class="update" onclick={updateNow} title="Pobierz i zainstaluj aktualizację" disabled={app.updating !== null}>{app.updating !== null ? `Pobieram… ${Math.round(app.updating * 100)}%` : `Aktualizuj do ${app.update.latest}`}</button>
  {/if}

  <div class="win">
    <button onclick={() => win.minimize()} title="Minimalizuj"><Icon name="min" size={15} /></button>
    <button onclick={() => win.toggleMaximize()} title={maximized ? 'Przywróć' : 'Maksymalizuj'}>
      <Icon name={maximized ? 'restore' : 'max'} size={13} />
    </button>
    <button class="close" onclick={() => win.close()} title="Zamknij"><Icon name="close" size={15} /></button>
  </div>
</header>

<style>
  .titlebar {
    height: 42px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-left: 8px;
    background: var(--bg);
    flex: none;
  }
  .side {
    margin-right: 2px;
  }
  .tabs {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    height: 100%;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabs::-webkit-scrollbar {
    display: none;
  }
  .tab {
    height: 32px;
    flex: 0 1 210px;
    min-width: 92px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 6px 0 12px;
    border-radius: 10px;
    color: var(--faint);
    transition: background 0.12s, color 0.12s;
    outline: none;
  }
  .tab.pinned {
    flex: none;
    min-width: 0;
    width: 40px;
    padding: 0;
    justify-content: center;
    position: relative;
  }
  .tab.pinned::after {
    content: '';
    position: absolute;
    top: 6px;
    right: 6px;
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--faint);
  }
  .tab:hover {
    background: var(--hover);
    color: var(--dim);
  }
  .tab.visible {
    color: var(--dim);
    background: var(--card);
  }
  .tab.active {
    background: var(--higher);
    color: var(--text);
  }
  .tab.over {
    background: var(--sel-strong);
    color: var(--text);
  }
  .ti {
    display: flex;
    opacity: 0.85;
  }
  .pane-tag {
    font-size: 10px;
    font-weight: 700;
    color: var(--faint);
  }
  .x {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    color: var(--dim);
  }
  .tab:hover .x,
  .tab.active .x {
    opacity: 1;
  }
  .x:hover {
    background: var(--press);
    color: var(--text);
  }
  .add {
    width: 30px;
    height: 30px;
  }
  .update {
    font-size: 12px;
    color: var(--on-accent);
    background: var(--accent);
    border-radius: 999px;
    padding: 4px 12px;
    text-decoration: none;
    font-weight: 600;
    white-space: nowrap;
  }
  .win {
    display: flex;
    height: 100%;
    flex: none;
  }
  .win button {
    width: 46px;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--dim);
  }
  .win button:hover {
    background: var(--hover);
    color: var(--text);
  }
  .win .close:hover {
    background: #e81123;
    color: #fff;
  }
</style>
