<script lang="ts">
  import Icon from './Icon.svelte'
  import Thumb from './Thumb.svelte'
  import {
    app, settings, openEntry, openViewer, navigate, newTab, transfer, dropMode, copySel, paste, trash, doRename,
    newFolder, newFile, addFavorite, load, savePrefs, toast, activate, closeSearch, type Tab, type MenuItem, type SortKey,
  } from '../lib/state.svelte'
  import { api, type Entry } from '../lib/api'
  import { date, size, typeLabel, kindOf, isMedia, parent, items as itemsLabel } from '../lib/util'

  let { tab, pane }: { tab: Tab; pane: number } = $props()

  let scroller = $state<HTMLDivElement>()
  let width = $state(800)
  let height = $state(600)
  let scrollTop = $state(0)

  const LIST_H = 32
  const list = $derived(tab.items)
  const view = $derived(tab.view)
  const gap = $derived(view === 'gallery' ? 4 : 6)
  const pad = $derived(view === 'list' ? 6 : 12)
  const cellW = $derived(view === 'grid' ? settings.gridSize : view === 'gallery' ? settings.galleryRow : width)
  const cols = $derived(view === 'list' ? 1 : Math.max(1, Math.floor((width - pad * 2 + gap) / (cellW + gap))))
  // Kafelki rozciągamy, żeby wypełniały szerokość bez pustego marginesu po prawej.
  const tileW = $derived(view === 'list' ? width - pad * 2 : Math.floor((width - pad * 2 - gap * (cols - 1)) / cols))
  const tileH = $derived(view === 'list' ? LIST_H : view === 'grid' ? tileW + 40 : tileW)
  const rowH = $derived(tileH + (view === 'list' ? 0 : gap))
  const top0 = $derived(view === 'list' ? 4 : pad)
  const rows = $derived(Math.ceil(list.length / cols))
  const totalH = $derived(top0 + rows * rowH + pad)
  const first = $derived(Math.max(0, Math.floor((scrollTop - top0) / rowH) - 3))
  const last = $derived(Math.min(rows, Math.ceil((scrollTop - top0 + height) / rowH) + 3))
  const visible = $derived.by(() => {
    const out: { e: Entry; i: number; x: number; y: number }[] = []
    for (let r = first; r < last; r++) {
      for (let c = 0; c < cols; c++) {
        const i = r * cols + c
        if (i >= list.length) break
        out.push({ e: list[i], i, x: pad + c * (tileW + gap), y: top0 + r * rowH })
      }
    }
    return out
  })
  const thumbPx = $derived(view === 'list' ? 20 : tileW)

  // Pamięć przewinięcia przy powrocie do folderu.
  $effect(() => {
    tab.path
    tab.search
    const saved = tab.scrollMemory.get(tab.path.toLowerCase()) ?? 0
    queueMicrotask(() => {
      if (scroller) scroller.scrollTop = saved
    })
  })

  $effect(() => {
    if (pane === app.focusedPane && scroller && !app.dialog && !app.viewer && document.activeElement?.tagName !== 'INPUT') {
      tab.path
      scroller.focus({ preventScroll: true })
    }
  })

  function onScroll() {
    scrollTop = scroller!.scrollTop
    tab.scrollTop = scrollTop
  }

  function ensureVisible(i: number) {
    if (!scroller || i < 0) return
    const r = Math.floor(i / cols)
    const y = top0 + r * rowH
    if (y < scroller.scrollTop) scroller.scrollTop = y - pad
    else if (y + tileH > scroller.scrollTop + height) scroller.scrollTop = y + tileH - height + pad
  }

  // ---------- Zaznaczanie ----------

  function selectRange(a: number, b: number, add: boolean) {
    if (!add) tab.selected.clear()
    const [s, e] = a < b ? [a, b] : [b, a]
    for (let i = s; i <= e; i++) if (list[i]) tab.selected.add(list[i].path)
  }

  let pendingSingle: number | null = null

  function itemDown(ev: MouseEvent, e: Entry, i: number) {
    activate(tab.id)
    app.focusedPane = pane
    if (ev.button === 2) {
      if (!tab.selected.has(e.path)) {
        tab.selected.clear()
        tab.selected.add(e.path)
        tab.focus = i
        tab.anchor = i
      }
      return
    }
    if (ev.button !== 0) return
    if (ev.shiftKey) {
      selectRange(tab.anchor >= 0 ? tab.anchor : i, i, ev.ctrlKey)
    } else if (ev.ctrlKey) {
      if (tab.selected.has(e.path)) tab.selected.delete(e.path)
      else tab.selected.add(e.path)
      tab.anchor = i
    } else if (tab.selected.has(e.path)) {
      pendingSingle = i
    } else {
      tab.selected.clear()
      tab.selected.add(e.path)
      tab.anchor = i
    }
    tab.focus = i
  }

  function itemUp(ev: MouseEvent, e: Entry, i: number) {
    if (pendingSingle === i && !ev.ctrlKey && !ev.shiftKey) {
      tab.selected.clear()
      tab.selected.add(e.path)
      tab.anchor = i
    }
    pendingSingle = null
    if (settings.singleClickOpen && ev.button === 0 && !ev.ctrlKey && !ev.shiftKey) openEntry(tab, e)
  }

  // Zaznaczanie ramką
  let marquee = $state<{ x0: number; y0: number; x1: number; y1: number } | null>(null)
  let marqueeBase: Set<string> = new Set()

  function bgDown(ev: MouseEvent, force = false) {
    activate(tab.id)
    app.focusedPane = pane
    if (!force && (ev.button !== 0 || ev.target !== ev.currentTarget && !(ev.target as HTMLElement).classList.contains('spacer'))) return
    const r = scroller!.getBoundingClientRect()
    if (ev.clientX > r.right - 12) return // pasek przewijania
    const x = ev.clientX - r.left
    const y = ev.clientY - r.top + scroller!.scrollTop
    marquee = { x0: x, y0: y, x1: x, y1: y }
    marqueeBase = ev.ctrlKey || ev.shiftKey ? new Set(tab.selected) : new Set()
    if (!ev.ctrlKey && !ev.shiftKey) tab.selected.clear()
    window.addEventListener('mousemove', bgMove)
    window.addEventListener('mouseup', bgUp, { once: true })
  }
  let autoScroll: ReturnType<typeof setInterval> | undefined
  let lastMove: MouseEvent | null = null
  function bgMove(ev: MouseEvent) {
    if (!marquee || !scroller) return
    lastMove = ev
    const r = scroller.getBoundingClientRect()
    marquee.x1 = Math.max(0, Math.min(width, ev.clientX - r.left))
    marquee.y1 = Math.max(0, ev.clientY - r.top + scroller.scrollTop)
    clearInterval(autoScroll)
    if (ev.clientY > r.bottom - 24 || ev.clientY < r.top + 24) {
      autoScroll = setInterval(() => {
        scroller!.scrollTop += ev.clientY > r.bottom - 24 ? 18 : -18
        if (lastMove) bgMove(lastMove)
      }, 30)
    }
    const x0 = Math.min(marquee.x0, marquee.x1), x1 = Math.max(marquee.x0, marquee.x1)
    const y0 = Math.min(marquee.y0, marquee.y1), y1 = Math.max(marquee.y0, marquee.y1)
    const sel = new Set(marqueeBase)
    const r0 = Math.max(0, Math.floor((y0 - top0) / rowH))
    const r1 = Math.min(rows - 1, Math.floor((y1 - top0) / rowH))
    for (let row = r0; row <= r1; row++) {
      for (let c = 0; c < cols; c++) {
        const i = row * cols + c
        if (i >= list.length) break
        const ix = pad + c * (tileW + gap), iy = top0 + row * rowH
        const iw = view === 'list' ? Math.min(tileW, 420) : tileW
        if (ix < x1 && ix + iw > x0 && iy < y1 && iy + tileH > y0) sel.add(list[i].path)
      }
    }
    tab.selected.clear()
    for (const s of sel) tab.selected.add(s)
  }
  function bgUp() {
    clearInterval(autoScroll)
    marquee = null
    window.removeEventListener('mousemove', bgMove)
  }

  // ---------- Przeciąganie ----------

  let dropTarget = $state<string | null>(null)
  function dragStart(ev: DragEvent, e: Entry) {
    if (!tab.selected.has(e.path)) {
      tab.selected.clear()
      tab.selected.add(e.path)
    }
    const paths = tab.selectedEntries.map((x) => x.path)
    app.dragging = { paths }
    ev.dataTransfer!.effectAllowed = 'copyMove'
    ev.dataTransfer!.setData('text/plain', paths.join('\n'))
    const ghost = document.createElement('div')
    ghost.className = 'drag-ghost'
    ghost.textContent = paths.length === 1 ? e.name : `${paths.length} elementów`
    document.body.appendChild(ghost)
    ev.dataTransfer!.setDragImage(ghost, -12, -8)
    setTimeout(() => ghost.remove(), 0)
  }
  function dragEnd() {
    app.dragging = null
    dropTarget = null
  }
  function itemDragOver(ev: DragEvent, e: Entry) {
    if (!app.dragging || !e.dir || app.dragging.paths.includes(e.path)) return
    ev.preventDefault()
    ev.stopPropagation()
    dropTarget = e.path
    ev.dataTransfer!.dropEffect = dropMode(app.dragging.paths, e.path, ev) === 'copy' ? 'copy' : 'move'
  }
  function itemDrop(ev: DragEvent, e: Entry) {
    if (!app.dragging || !e.dir) return
    ev.preventDefault()
    ev.stopPropagation()
    dropTarget = null
    transfer(app.dragging.paths, e.path, dropMode(app.dragging.paths, e.path, ev))
  }
  function bgDragOver(ev: DragEvent) {
    if (!app.dragging || tab.special || tab.search) return
    if (app.dragging.paths.every((p) => parent(p)?.toLowerCase() === tab.path.toLowerCase())) return
    ev.preventDefault()
    dropTarget = '#bg'
    ev.dataTransfer!.dropEffect = dropMode(app.dragging.paths, tab.path, ev) === 'copy' ? 'copy' : 'move'
  }
  function bgDrop(ev: DragEvent) {
    ev.preventDefault()
    dropTarget = null
    if (app.dragging) transfer(app.dragging.paths, tab.path, dropMode(app.dragging.paths, tab.path, ev))
  }

  // ---------- Klawiatura ----------

  let typed = ''
  let typedAt = 0
  function onKey(ev: KeyboardEvent) {
    if (tab.renaming || app.dialog || app.viewer) return
    const n = list.length
    let f = tab.focus
    const move = (to: number) => {
      to = Math.max(0, Math.min(n - 1, to))
      if (ev.shiftKey) selectRange(tab.anchor >= 0 ? tab.anchor : to, to, false)
      else if (!ev.ctrlKey) {
        tab.selected.clear()
        tab.selected.add(list[to].path)
        tab.anchor = to
      }
      tab.focus = to
      ensureVisible(to)
      ev.preventDefault()
    }
    const pageRows = Math.max(1, Math.floor(height / rowH) - 1)
    switch (ev.key) {
      case 'ArrowDown': return n && move(f < 0 ? 0 : f + cols)
      case 'ArrowUp': return n && move(f < 0 ? 0 : f - cols)
      case 'ArrowRight': if (view !== 'list' && !ev.altKey) return n && move(f + 1); break
      case 'ArrowLeft': if (view !== 'list' && !ev.altKey) return n && move(f - 1); break
      case 'PageDown': return n && move(f + pageRows * cols)
      case 'PageUp': return n && move(f - pageRows * cols)
      case 'Home': return n && move(0)
      case 'End': return n && move(n - 1)
      case 'Enter':
        if (ev.altKey) return
        ev.preventDefault()
        if (tab.selectedEntries.length > 1 && tab.selectedEntries.every((e) => !e.dir)) {
          tab.selectedEntries.slice(0, 20).forEach((e) => api.open(e.path))
        } else if (f >= 0 && list[f]) openEntry(tab, list[f], ev.ctrlKey)
        return
      case ' ':
        ev.preventDefault()
        if (f >= 0 && list[f] && !list[f].dir) openViewer(tab, list[f])
        return
      case 'F2':
        ev.preventDefault()
        if (tab.selectedEntries.length === 1) tab.renaming = tab.selectedEntries[0].path
        return
      case 'Delete':
        ev.preventDefault()
        trash(tab, ev.shiftKey)
        return
      case 'Escape':
        if (tab.search) closeSearch(tab)
        else tab.selected.clear()
        return
    }
    if (ev.ctrlKey && !ev.altKey) {
      const k = ev.key.toLowerCase()
      if (k === 'a') {
        ev.preventDefault()
        list.forEach((e) => tab.selected.add(e.path))
      } else if (k === 'c' && !ev.shiftKey) {
        ev.preventDefault()
        copySel(tab, false)
      } else if (k === 'c' && ev.shiftKey) {
        ev.preventDefault()
        navigator.clipboard.writeText(tab.selectedEntries.map((e) => `"${e.path}"`).join('\n') || tab.path)
        toast('Skopiowano ścieżkę')
      } else if (k === 'x') {
        ev.preventDefault()
        copySel(tab, true)
      } else if (k === 'v') {
        ev.preventDefault()
        paste(tab)
      } else if (k === 'i') {
        ev.preventDefault()
        const all = new Set(list.map((e) => e.path))
        for (const s of tab.selected) all.delete(s)
        tab.selected.clear()
        all.forEach((p) => tab.selected.add(p))
      }
      return
    }
    // Pisanie przeskakuje do pasującej nazwy.
    if (ev.key.length === 1 && !ev.altKey && !ev.ctrlKey && !ev.metaKey) {
      const now = Date.now()
      typed = now - typedAt > 800 ? ev.key.toLowerCase() : typed + ev.key.toLowerCase()
      typedAt = now
      const start = typed.length === 1 ? f + 1 : Math.max(0, f)
      for (let k = 0; k < n; k++) {
        const i = (start + k) % n
        if (list[i].name.toLowerCase().startsWith(typed)) {
          tab.selected.clear()
          tab.selected.add(list[i].path)
          tab.focus = i
          tab.anchor = i
          ensureVisible(i)
          break
        }
      }
    }
  }

  // Ctrl + kółko zmienia wielkość miniatur.
  function onWheel(ev: WheelEvent) {
    if (!ev.ctrlKey) return
    ev.preventDefault()
    const d = ev.deltaY < 0 ? 12 : -12
    if (view === 'list') {
      if (d > 0) tab.view = 'grid'
    } else if (view === 'grid') {
      settings.gridSize = Math.max(84, Math.min(280, settings.gridSize + d))
      if (settings.gridSize <= 84 && d < 0) tab.view = 'list'
    } else settings.galleryRow = Math.max(120, Math.min(420, settings.galleryRow + d))
    savePrefs(tab)
  }

  // ---------- Menu kontekstowe ----------

  function native(paths: string[], ev: MouseEvent) {
    api.nativeMenu(paths, ev.clientX, ev.clientY, ev.shiftKey).then((done) => done && setTimeout(() => load(tab, true), 400)).catch((err) => toast(String(err), 'error'))
  }

  function itemMenu(ev: MouseEvent, e: Entry) {
    ev.preventDefault()
    const sel = tab.selectedEntries.length ? tab.selectedEntries : [e]
    // Shift + prawy przycisk: od razu pełne menu Eksploratora.
    if (ev.shiftKey) return native(sel.map((x) => x.path), ev)
    const one = sel.length === 1 ? sel[0] : null
    const other = app.split ? app.tabs.find((t) => t.id === app.panes[1 - pane]) : null
    const items: MenuItem[] = []
    if (one?.dir) {
      items.push(
        { label: 'Otwórz', icon: 'folder', kbd: 'Enter', run: () => navigate(tab, one.path) },
        { label: 'Otwórz w nowej karcie', icon: 'plus', kbd: 'Ctrl+Enter', run: () => newTab(one.path, false, tab.id) },
      )
    } else if (one) {
      items.push({ label: 'Otwórz', icon: 'open', kbd: 'Enter', run: () => openEntry(tab, one) })
      items.push({ label: 'Podgląd', icon: 'eye', kbd: 'Spacja', run: () => openViewer(tab, one) })
      items.push({ label: 'Otwórz za pomocą…', icon: 'app', run: () => api.openWith(one.path) })
    }
    if (tab.search && one) items.push({ label: 'Przejdź do lokalizacji', icon: 'folder', run: () => { const p = parent(one.path); if (p) { navigate(tab, p) } } })
    items.push({ sep: true })
    items.push({ label: 'Wytnij', icon: 'cut', kbd: 'Ctrl+X', run: () => copySel(tab, true) })
    items.push({ label: 'Kopiuj', icon: 'copy', kbd: 'Ctrl+C', run: () => copySel(tab, false) })
    if (one?.dir) items.push({ label: 'Wklej do folderu', icon: 'paste', run: () => paste(tab, one.path) })
    items.push({ label: 'Kopiuj ścieżkę', icon: 'link', kbd: 'Ctrl+Shift+C', run: () => { navigator.clipboard.writeText(sel.map((x) => `"${x.path}"`).join('\n')); toast('Skopiowano ścieżkę') } })
    if (other && !other.special) {
      items.push({
        label: `Do drugiego panelu`, icon: 'split',
        sub: [
          { label: `Kopiuj do „${other.title}”`, icon: 'copy', run: () => transfer(sel.map((x) => x.path), other.path, 'copy') },
          { label: `Przenieś do „${other.title}”`, icon: 'cut', run: () => transfer(sel.map((x) => x.path), other.path, 'move') },
        ],
      })
    }
    items.push({ sep: true })
    if (one) items.push({ label: 'Zmień nazwę', icon: 'edit', kbd: 'F2', run: () => (tab.renaming = one.path) })
    items.push({ label: 'Do Kosza', icon: 'trash', kbd: 'Del', run: () => trash(tab) })
    items.push({ label: 'Usuń trwale', icon: 'trash', kbd: 'Shift+Del', danger: true, run: () => trash(tab, true) })
    items.push({ sep: true })
    if (one?.dir) {
      items.push({ label: 'Przypnij w panelu bocznym', icon: 'pin', run: () => addFavorite(one.path) })
      items.push({ label: 'Synchronizuj z dyskiem…', icon: 'sync', run: () => (app.dialog = { type: 'sync', profile: null, source: one.path }) })
      items.push({ label: 'Terminal tutaj', icon: 'terminal', run: () => api.terminal(one.path) })
    }
    items.push({ label: 'Pokaż w Eksploratorze', icon: 'open', run: () => api.reveal(sel[0].path) })
    items.push({ sep: true })
    items.push({ label: 'Właściwości', icon: 'info', kbd: 'Alt+Enter', run: () => (app.dialog = { type: 'props', entries: sel }) })
    items.push({ sep: true })
    items.push({ label: 'Pokaż więcej opcji (menu Windows)', icon: 'more', kbd: 'Shift+PPM', run: () => native(sel.map((x) => x.path), ev) })
    if (one) items.push({ label: 'Właściwości systemowe', icon: 'settings', run: () => api.systemProps(one.path) })
    app.menu = { x: ev.clientX, y: ev.clientY, items }
  }

  function bgMenu(ev: MouseEvent) {
    if (ev.target !== ev.currentTarget && !(ev.target as HTMLElement).classList.contains('spacer')) return
    ev.preventDefault()
    tab.selected.clear()
    if (ev.shiftKey && !tab.special) return native([tab.path], ev)
    const sortItem = (k: SortKey, label: string) => ({ label: (tab.sort === k ? (tab.desc ? '↓ ' : '↑ ') : '   ') + label, run: () => { if (tab.sort === k) tab.desc = !tab.desc; else tab.sort = k; savePrefs(tab) } })
    app.menu = {
      x: ev.clientX, y: ev.clientY,
      items: [
        { label: 'Nowy folder', icon: 'folderPlus', kbd: 'Ctrl+Shift+N', run: () => newFolder(tab) },
        { label: 'Nowy plik tekstowy', icon: 'filePlus', run: () => newFile(tab) },
        { label: 'Wklej', icon: 'paste', kbd: 'Ctrl+V', run: () => paste(tab) },
        { sep: true },
        { label: 'Widok', icon: 'grid', sub: [
          { label: 'Lista', icon: 'list', kbd: 'Ctrl+1', run: () => { tab.view = 'list'; savePrefs(tab) } },
          { label: 'Siatka', icon: 'grid', kbd: 'Ctrl+2', run: () => { tab.view = 'grid'; savePrefs(tab) } },
          { label: 'Galeria', icon: 'gallery', kbd: 'Ctrl+3', run: () => { tab.view = 'gallery'; savePrefs(tab) } },
        ] },
        { label: 'Sortuj według', icon: 'sort', sub: [sortItem('name', 'Nazwa'), sortItem('modified', 'Data modyfikacji'), sortItem('size', 'Rozmiar'), sortItem('type', 'Typ'), sortItem('created', 'Data utworzenia')] },
        { label: 'Odśwież', icon: 'refresh', kbd: 'F5', run: () => load(tab, true) },
        { label: 'Zaznacz wszystko', icon: 'check', kbd: 'Ctrl+A', run: () => list.forEach((e) => tab.selected.add(e.path)) },
        { sep: true },
        { label: 'Przypnij ten folder', icon: 'pin', run: () => addFavorite(tab.path) },
        { label: 'Synchronizuj ten folder…', icon: 'sync', run: () => (app.dialog = { type: 'sync', profile: null, source: tab.path }) },
        { label: 'Terminal tutaj', icon: 'terminal', run: () => api.terminal(tab.path) },
        { label: 'Otwórz w Eksploratorze', icon: 'open', run: () => api.open(tab.path) },
        { sep: true },
        { label: 'Właściwości folderu', icon: 'info', run: () => api.stat(tab.path).then((e) => (app.dialog = { type: 'props', entries: [e] })) },
        { sep: true },
        { label: 'Pokaż więcej opcji (menu Windows)', icon: 'more', run: () => native([tab.path], ev) },
      ],
    }
  }

  // ---------- Zmiana nazwy ----------

  function renameMount(node: HTMLInputElement, e: Entry) {
    node.focus()
    const dot = e.dir ? -1 : e.name.lastIndexOf('.')
    node.setSelectionRange(0, dot > 0 ? dot : e.name.length)
  }
  function renameKey(ev: KeyboardEvent, e: Entry) {
    ev.stopPropagation()
    if (ev.key === 'Enter') doRename(tab, e, (ev.target as HTMLInputElement).value)
    else if (ev.key === 'Escape') tab.renaming = null
    else if (ev.key === 'Tab') {
      // Tab przechodzi do zmiany nazwy następnego elementu.
      ev.preventDefault()
      const i = list.findIndex((x) => x.path === e.path)
      doRename(tab, e, (ev.target as HTMLInputElement).value).then(() => {
        const nx = list[i + (ev.shiftKey ? -1 : 1)]
        if (nx) tab.renaming = nx.path
      })
    }
  }

  function setSort(k: SortKey) {
    if (tab.sort === k) tab.desc = !tab.desc
    else {
      tab.sort = k
      tab.desc = k !== 'name' && k !== 'type'
    }
    savePrefs(tab)
  }

  const marqueeStyle = $derived(
    marquee
      ? `left:${Math.min(marquee.x0, marquee.x1)}px;top:${Math.min(marquee.y0, marquee.y1)}px;width:${Math.abs(marquee.x1 - marquee.x0)}px;height:${Math.abs(marquee.y1 - marquee.y0)}px`
      : '',
  )
</script>

<div class="wrap">
  {#if view === 'list'}
    <div class="header">
      <button class="c name" onclick={() => setSort('name')}>Nazwa {tab.sort === 'name' ? (tab.desc ? '↓' : '↑') : ''}</button>
      {#if tab.search}<span class="c loc">Lokalizacja</span>{/if}
      <button class="c mod" onclick={() => setSort('modified')}>Zmodyfikowano {tab.sort === 'modified' ? (tab.desc ? '↓' : '↑') : ''}</button>
      <button class="c type" onclick={() => setSort('type')}>Typ {tab.sort === 'type' ? (tab.desc ? '↓' : '↑') : ''}</button>
      <button class="c size" onclick={() => setSort('size')}>Rozmiar {tab.sort === 'size' ? (tab.desc ? '↓' : '↑') : ''}</button>
    </div>
  {/if}
<div
  class="view {view}"
  class:drop={dropTarget === '#bg'}
  bind:this={scroller}
  bind:clientWidth={width}
  bind:clientHeight={height}
  onscroll={onScroll}
  onmousedown={bgDown}
  oncontextmenu={bgMenu}
  onkeydown={onKey}
  onwheel={onWheel}
  ondragover={bgDragOver}
  ondragleave={(e) => { if (e.target === e.currentTarget) dropTarget = null }}
  ondrop={bgDrop}
  tabindex="0"
  role="listbox"
  aria-multiselectable="true"
>
  <div class="spacer" style:height="{totalH}px"></div>


  {#each visible as { e, i, x, y } (e.path)}
    {@const sel = tab.selected.has(e.path)}
    <div
      class="item"
      class:sel
      class:focus={tab.focus === i && pane === app.focusedPane}
      class:hidden={e.hidden}
      class:cut={false}
      class:droptarget={dropTarget === e.path}
      style="transform:translate({x}px,{y}px);width:{tileW}px;height:{tileH}px"
      role="option"
      tabindex="-1"
      aria-selected={sel}
      draggable={tab.renaming !== e.path && view !== 'list'}
      onmousedown={(ev) => {
        ev.stopPropagation()
        // Na liście ramkę zaczyna się poza nazwą (jak w Eksploratorze); w siatce/galerii z Altem.
        const grab = (ev.target as HTMLElement).closest('.name, .ic, .thumb img, .label, input')
        if (ev.button === 0 && ((view === 'list' && !grab) || ev.altKey)) { ev.preventDefault(); bgDown(ev, true); return }
        itemDown(ev, e, i)
      }}
      onmouseup={(ev) => itemUp(ev, e, i)}
      ondblclick={() => !settings.singleClickOpen && openEntry(tab, e)}
      onauxclick={(ev) => ev.button === 1 && e.dir && newTab(e.path, false, tab.id)}
      oncontextmenu={(ev) => { ev.stopPropagation(); itemMenu(ev, e) }}
      ondragstart={(ev) => dragStart(ev, e)}
      ondragend={dragEnd}
      ondragover={(ev) => itemDragOver(ev, e)}
      ondragleave={() => dropTarget === e.path && (dropTarget = null)}
      ondrop={(ev) => itemDrop(ev, e)}
      title={view === 'list' ? undefined : `${e.name}\n${e.dir ? 'Folder' : size(e.size)} · ${date(e.modified)}`}
    >
      {#if view === 'list'}
        <span class="ic" draggable="true"><Thumb entry={e} px={20} iconOnly pad={false} stamp={tab.stamp} /></span>
        {#if tab.renaming === e.path}
          <input class="rename" value={e.name} use:renameMount={e} onkeydown={(ev) => renameKey(ev, e)} onblur={(ev) => doRename(tab, e, ev.currentTarget.value)} onmousedown={(ev) => ev.stopPropagation()} />
        {:else}
          <span class="c name ellipsis" draggable="true">{e.name}{#if e.cloud}<span class="cloud" title="Tylko w chmurze"><Icon name="cloud" size={13} /></span>{/if}</span>
        {/if}
        {#if tab.search}<span class="c loc ellipsis faint">{parent(e.path)}</span>{/if}
        <span class="c mod">{date(e.modified)}</span>
        <span class="c type ellipsis">{typeLabel(e)}</span>
        <span class="c size">{e.dir ? '' : size(e.size)}</span>
      {:else}
        <div class="thumb" class:media={isMedia(e)} class:folder={e.dir}>
          <Thumb entry={e} px={thumbPx} fit={view === 'gallery' ? 'cover' : 'contain'} iconOnly={e.dir} stamp={tab.stamp} />
          {#if kindOf(e) === 'video'}<span class="badge"><Icon name="play" size={11} fill /></span>{/if}
          {#if view === 'gallery' && (e.dir || !isMedia(e))}<span class="gname ellipsis">{e.name}</span>{/if}
          {#if sel && view === 'gallery'}<span class="check"><Icon name="check" size={13} /></span>{/if}
        </div>
        {#if view === 'grid'}
          {#if tab.renaming === e.path}
            <input class="rename grid" value={e.name} use:renameMount={e} onkeydown={(ev) => renameKey(ev, e)} onblur={(ev) => doRename(tab, e, ev.currentTarget.value)} onmousedown={(ev) => ev.stopPropagation()} />
          {:else}
            <span class="label">{e.name}</span>
          {/if}
        {/if}
      {/if}
    </div>
  {/each}

  {#if marquee}<div class="marquee" style={marqueeStyle}></div>{/if}

  {#if !tab.loading && !list.length}
    <div class="empty" style:top="{scrollTop}px">
      {#if tab.error}
        <Icon name="info" size={28} />
        <p>{tab.error}</p>
        <button class="btn" onclick={() => load(tab)}>Spróbuj ponownie</button>
      {:else if tab.search}
        {#if tab.search.running}<p class="muted">Szukam… przejrzano {itemsLabel(tab.search.scanned)}</p>
        {:else}<p class="muted">Nic nie znaleziono dla „{tab.search.query}”</p>{/if}
      {:else if tab.filter || tab.kindFilter !== 'all'}
        <p class="muted">Brak pasujących elementów</p>
      {:else}
        <Icon name="folder" size={30} />
        <p class="muted">Ten folder jest pusty</p>
      {/if}
    </div>
  {/if}
</div>
</div>

<style>
  .wrap {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .view {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    outline: none;
    contain: strict;
  }
  .view.drop {
    background: rgba(255, 255, 255, 0.03);
    box-shadow: inset 0 0 0 2px var(--line-strong);
    border-radius: var(--r);
  }
  .spacer {
    width: 1px;
  }
  .item {
    position: absolute;
    left: 0;
    top: 0;
    border-radius: 10px;
    will-change: transform;
    contain: layout style;
  }
  .item.hidden {
    opacity: 0.55;
  }

  /* Lista */
  .header {
    flex: none;
    height: 30px;
    display: flex;
    align-items: center;
    padding: 0 18px 0 46px;
    background: var(--bg);
    color: var(--faint);
    font-size: 12px;
  }
  .header .c {
    text-align: left;
    padding: 4px 6px;
    border-radius: 6px;
  }
  .header button.c:hover {
    background: var(--hover);
    color: var(--text);
  }
  .list .item {
    display: flex;
    align-items: center;
    gap: 0;
    padding: 0 6px 0 10px;
    color: var(--dim);
  }
  .list .item:hover {
    background: var(--hover);
  }
  .list .item.sel {
    background: var(--sel);
    color: var(--text);
  }
  .list .item.focus {
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .list .item.droptarget {
    background: var(--sel-strong);
    box-shadow: inset 0 0 0 1px #fff;
  }
  .ic {
    width: 20px;
    height: 20px;
    flex: none;
    margin-right: 10px;
  }
  .c {
    padding: 0 6px;
    flex: none;
  }
  .c.name {
    flex: 1;
    min-width: 120px;
    color: var(--text);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .c.loc {
    width: 26%;
    font-size: 12px;
  }
  .c.mod {
    width: 140px;
  }
  .c.type {
    width: 130px;
  }
  .c.size {
    width: 86px;
    text-align: right;
  }
  .header .c.size {
    text-align: right;
  }
  .cloud {
    color: var(--info);
    display: inline-flex;
  }
  .rename {
    flex: 1;
    height: 26px;
    padding: 2px 8px;
    margin-right: 6px;
    user-select: text;
  }

  /* Siatka */
  .grid .item {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 6px;
    gap: 6px;
  }
  .grid .item:hover {
    background: var(--hover);
  }
  .grid .item.sel {
    background: var(--sel);
  }
  .grid .item.focus {
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .grid .item.droptarget {
    background: var(--sel-strong);
    box-shadow: inset 0 0 0 1px #fff;
  }
  .grid .thumb {
    position: relative;
    width: 100%;
    flex: 1;
    min-height: 0;
    border-radius: 8px;
    overflow: hidden;
  }
  .grid .thumb.media {
    background: var(--panel);
  }
  .label {
    width: 100%;
    font-size: 12px;
    line-height: 15px;
    height: 30px;
    text-align: center;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    word-break: break-word;
    color: var(--dim);
  }
  .item.sel .label {
    color: var(--text);
  }
  .rename.grid {
    width: 100%;
    height: 30px;
    font-size: 12px;
    text-align: center;
    margin: 0;
  }

  /* Galeria */
  .gallery .item {
    overflow: hidden;
    border-radius: 6px;
  }
  .gallery .thumb {
    position: relative;
    width: 100%;
    height: 100%;
    background: var(--panel);
    transition: transform 0.2s var(--ease);
  }
  .gallery .thumb.folder {
    background: var(--high);
  }
  .gallery .item:hover .thumb {
    filter: brightness(1.12);
  }
  .gallery .item.sel .thumb {
    transform: scale(0.9);
    border-radius: 8px;
    overflow: hidden;
  }
  .gallery .item.sel {
    background: var(--sel);
  }
  .gallery .item.focus {
    box-shadow: inset 0 0 0 2px var(--line-strong);
  }
  .gallery .item.droptarget {
    box-shadow: inset 0 0 0 2px #fff;
  }
  .gname {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    padding: 18px 10px 8px;
    font-size: 12px;
    background: linear-gradient(transparent, rgba(0, 0, 0, 0.85));
  }
  .check {
    position: absolute;
    top: 8px;
    left: 8px;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: #fff;
    color: #000;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .badge {
    position: absolute;
    right: 6px;
    bottom: 6px;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.6);
    color: #fff;
    display: flex;
    align-items: center;
    justify-content: center;
    padding-left: 2px;
  }

  .marquee {
    position: absolute;
    z-index: 4;
    background: rgba(255, 255, 255, 0.07);
    border: 1px solid rgba(255, 255, 255, 0.45);
    border-radius: 4px;
    pointer-events: none;
  }
  .empty {
    position: absolute;
    left: 0;
    right: 0;
    height: 70%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    color: var(--faint);
    pointer-events: none;
    text-align: center;
    padding: 0 30px;
  }
  .empty .btn {
    pointer-events: auto;
  }
  :global(.drag-ghost) {
    position: fixed;
    top: -100px;
    padding: 6px 12px;
    background: #fff;
    color: #000;
    border-radius: 999px;
    font: 600 12px var(--font);
  }
</style>
