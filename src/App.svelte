<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import TitleBar from './components/TitleBar.svelte'
  import Sidebar from './components/Sidebar.svelte'
  import Pane from './components/Pane.svelte'
  import PreviewPane from './components/PreviewPane.svelte'
  import Viewer from './components/Viewer.svelte'
  import ContextMenu from './components/ContextMenu.svelte'
  import Dialogs from './components/Dialogs.svelte'
  import Overlay from './components/Overlay.svelte'
  import Palette from './components/Palette.svelte'
  import BulkRename from './components/BulkRename.svelte'
  import Shelf from './components/Shelf.svelte'
  import Terminal from './components/Terminal.svelte'
  import {
    app, settings, init, activeTab, tabById, newTab, closeTab, reopenClosed, activate, toggleSplit, back, forward, up, load,
    newFolder, undo, savePrefs, persistNow, shelfAdd,
  } from './lib/state.svelte'
  import { api } from './lib/api'

  let panes = $state<ReturnType<typeof Pane>[]>([])

  init().then(async () => {
    // Okno pokazujemy dopiero po pierwszym renderze – bez białego mignięcia.
    const hidden = await api.startHidden()
    if (!hidden) requestAnimationFrame(() => getCurrentWindow().show())
  })

  const focusedTab = $derived(app.ready ? activeTab() : null)
  let termUsed = $state(false)
  $effect(() => {
    if (settings.terminalOpen) termUsed = true
  })

  function onKey(ev: KeyboardEvent) {
    if (app.viewer || app.palette) return
    const t = activeTab()
    if (!t) return
    const inInput = (ev.target as HTMLElement)?.tagName === 'INPUT' || (ev.target as HTMLElement)?.tagName === 'TEXTAREA'
    const k = ev.key.toLowerCase()
    const ctrl = ev.ctrlKey && !ev.altKey

    if (app.dialog) return
    if (ctrl && k === 't' && !ev.shiftKey) return stop(ev, () => newTab(t.special ? t.path : t.path, true, t.id))
    if (ctrl && k === 't' && ev.shiftKey) return stop(ev, reopenClosed)
    if (ctrl && k === 'w') return stop(ev, () => closeTab(t.id))
    if (ctrl && k === 'tab') {
      return stop(ev, () => {
        const i = app.tabs.findIndex((x) => x.id === t.id)
        const n = app.tabs.length
        activate(app.tabs[(i + (ev.shiftKey ? -1 : 1) + n) % n].id)
      })
    }
    if (ctrl && k === 'k') return stop(ev, () => { app.paletteMode = 'all'; app.palette = true })
    if (ctrl && ev.shiftKey && k === 'p') return stop(ev, () => { app.paletteMode = 'cmd'; app.palette = true })
    if (ctrl && k === 'p' && !ev.shiftKey) return stop(ev, () => (settings.previewPane = !settings.previewPane))
    if (ctrl && ev.shiftKey && k === 's') return stop(ev, () => (t.selectedEntries.length ? shelfAdd(t.selectedEntries) : (settings.shelfOpen = !settings.shelfOpen)))
    if (ctrl && (ev.key === '`' || ev.code === 'Backquote')) return stop(ev, () => (settings.terminalOpen = !settings.terminalOpen))
    if (ctrl && k === 'b') return stop(ev, () => (app.sidebarOpen = !app.sidebarOpen))
    if (ctrl && k === '\\') return stop(ev, toggleSplit)
    // F6 / Shift+F6 przechodzi między obszarami: panel boczny → pasek adresu → pliki (→ drugi panel) → podgląd.
    if (ev.key === 'F6') return stop(ev, () => cycleRegion(ev.shiftKey ? -1 : 1))
    if (ctrl && k === 'l') return stop(ev, () => panes[app.focusedPane]?.focusAddress())
    if (ev.altKey && ev.key === 'd') return stop(ev, () => panes[app.focusedPane]?.focusAddress())
    if ((ctrl && k === 'f') || ev.key === 'F3') return stop(ev, () => panes[app.focusedPane]?.focusSearch())
    if (ev.key === 'F5') return stop(ev, () => load(t, true))
    if (ctrl && k === 'h') return stop(ev, () => (settings.showHidden = !settings.showHidden))
    if (ctrl && ev.shiftKey && k === 'n') return stop(ev, () => newFolder(t))
    if (ctrl && !ev.shiftKey && (ev.key === '1' || ev.key === '2' || ev.key === '3') && !t.special) {
      return stop(ev, () => {
        t.view = (['list', 'grid', 'gallery'] as const)[+ev.key - 1]
        savePrefs(t)
      })
    }
    if (inInput) return
    if (ctrl && k === 'z') return stop(ev, undo)
    if (ev.altKey && ev.key === 'ArrowLeft') return stop(ev, () => back(t))
    if (ev.altKey && ev.key === 'ArrowRight') return stop(ev, () => forward(t))
    if (ev.altKey && ev.key === 'ArrowUp') return stop(ev, () => up(t))
    if (ev.key === 'Backspace') return stop(ev, () => (t.search ? back(t) : up(t)))
    if (ev.altKey && ev.key === 'Enter' && t.selectedEntries.length) return stop(ev, () => (app.dialog = { type: 'props', entries: t.selectedEntries }))
  }

  function cycleRegion(dir: 1 | -1) {
    const regions: (() => HTMLElement | null)[] = [
      () => document.querySelector<HTMLElement>('[data-region="sidebar"] button.item'),
      () => (panes[app.focusedPane]?.focusAddress(), document.activeElement as HTMLElement),
      () => { app.focusedPane = 0; return document.querySelectorAll<HTMLElement>('[role="listbox"].view')[0] ?? null },
      () => { if (!app.split) return null; app.focusedPane = 1; return document.querySelectorAll<HTMLElement>('[role="listbox"].view')[1] ?? null },
      () => document.querySelector<HTMLElement>('[data-region="preview"]'),
    ]
    const act = document.activeElement as HTMLElement | null
    let cur = 2
    if (act?.closest('[data-region="sidebar"]')) cur = 0
    else if (act?.closest('.address')) cur = 1
    else if (act?.closest('[data-region="preview"]')) cur = 4
    else if (app.split && app.focusedPane === 1) cur = 3
    for (let k = 1; k <= regions.length; k++) {
      const i = (cur + dir * k + regions.length * 2) % regions.length
      if (i === 1) {
        regions[1]()
        return
      }
      const el = regions[i]()
      if (el) {
        el.focus()
        return
      }
    }
  }

  function stop(ev: KeyboardEvent, fn: () => void) {
    ev.preventDefault()
    ev.stopPropagation()
    fn()
  }

  // Boczne przyciski myszy: wstecz / dalej.
  function onMouse(ev: MouseEvent) {
    const t = activeTab()
    if (!t) return
    if (ev.button === 3) {
      ev.preventDefault()
      back(t)
    } else if (ev.button === 4) {
      ev.preventDefault()
      forward(t)
    }
  }

  function onContext(ev: MouseEvent) {
    // Systemowe menu WebView jest wyłączone – poza polami tekstowymi.
    const tag = (ev.target as HTMLElement)?.tagName
    if (tag !== 'INPUT' && tag !== 'TEXTAREA' && !(ev.target as HTMLElement)?.closest('pre, dd')) ev.preventDefault()
  }
</script>

<svelte:window onkeydown={onKey} onmouseup={onMouse} oncontextmenu={onContext} onbeforeunload={persistNow} />

<div class="app">
  <TitleBar />
  {#if app.ready}
    <div class="main">
      {#if app.sidebarOpen}<Sidebar />{/if}
      <div class="work">
        <div class="panes">
          {#each app.panes as id, i (id + ':' + i)}
            {@const t = tabById(id)}
            {#if t}<Pane bind:this={panes[i]} tab={t} pane={i} />{/if}
          {/each}
        </div>
        <!-- Terminal zostaje w tle po ukryciu – sesja i historia nie giną. -->
        {#if termUsed}<Terminal hidden={!settings.terminalOpen} />{/if}
      </div>
      {#if settings.previewPane && focusedTab && !focusedTab.special}
        <PreviewPane tab={focusedTab} />
      {/if}
    </div>
  {/if}
</div>

{#if app.viewer}<Viewer />{/if}
{#if app.menu}<ContextMenu items={app.menu.items} x={app.menu.x} y={app.menu.y} onclose={() => (app.menu = null)} />{/if}
{#if app.menu}<div class="menu-catch" role="presentation" onmousedown={() => (app.menu = null)} oncontextmenu={(e) => { e.preventDefault(); app.menu = null }}></div>{/if}
<Dialogs />
{#if settings.shelfOpen}<Shelf />{/if}
{#if app.dialog?.type === 'bulkRename'}<BulkRename entries={app.dialog.entries} />{/if}
{#if app.palette}<Palette />{/if}
<Overlay />

<style>
  .app {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 0;
  }
  .work {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .panes {
    min-height: 0;
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 6px;
    padding: 0 6px;
  }
  .menu-catch {
    position: fixed;
    inset: 0;
    z-index: 499;
  }
</style>
