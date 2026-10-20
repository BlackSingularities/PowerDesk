<script lang="ts">
  import Icon from './Icon.svelte'
  import Thumb from './Thumb.svelte'
  import {
    app, settings, openEntry, openViewer, navigate, newTab, transfer, dropMode, copySel, paste, trash, doRename,
    newFolder, newFile, shelfAdd, back, forward, openSpace, openShortcutTarget, extractHere, extractTo, zipSelection, newFolderWith, addFavorite, toggleHomeDir, isHomeDir, load, savePrefs, toast, activate, closeSearch, LIST_COLS, setSort as setSortOf, sortMenuItems, ensureMedia, mediaInfo, media, ensureDirSizes, dirSizes, dirSizeVer,
    type ListCol, type Tab, type MenuItem, type SortKey, type Group,
  } from '../lib/state.svelte'
  import { api, type Entry } from '../lib/api'
  import { date, size, typeLabel, kindOf, isMedia, parent, duration, items as itemsLabel, isBrowsableArchive, inArchive } from '../lib/util'

  let { tab, pane }: { tab: Tab; pane: number } = $props()

  let scroller = $state<HTMLDivElement>()
  let width = $state(800)
  let height = $state(600)
  let scrollTop = $state(0)

  const LIST_H = 32
  const HEAD_H = 38
  const list = $derived(tab.items)
  const view = $derived(tab.view)
  const gap = 6
  const pad = $derived(view === 'list' ? 6 : 12)
  const top0 = $derived(view === 'list' ? 4 : pad)
  const avail = $derived(Math.max(100, width - pad * 2))
  const cols = $derived(view === 'grid' ? Math.max(1, Math.floor((avail + gap) / (settings.gridSize + gap))) : 1)
  // Kafelki rozciągamy, żeby wypełniały szerokość bez pustego marginesu po prawej.
  const tileW = $derived(view === 'grid' ? Math.floor((avail - gap * (cols - 1)) / cols) : avail)
  const tileH = $derived(view === 'list' ? LIST_H : tileW + 40)

  // Metadane (wymiary, czas trwania) dla wszystkich widoków – partiami, od góry listy.
  $effect(() => {
    ensureMedia(list)
  })
  // Rozmiary folderów – tylko gdy kolumna rozmiaru jest widoczna albo sortujemy po rozmiarze.
  $effect(() => {
    if (settings.folderSizes && !tab.search && ((view === 'list' && settings.listCols.includes('size')) || tab.sort === 'size')) ensureDirSizes(list)
  })
  const dsize = (e: Entry) => {
    dirSizeVer.ver
    const d = dirSizes.get(e.path.toLowerCase())
    return d ? size(d.size) : settings.folderSizes ? '…' : ''
  }
  const info = (e: Entry) => (media.ver, mediaInfo.get(e.path))

  // ---------- Układ: lista, siatka i galeria (wyjustowane wiersze) z nagłówkami grup ----------

  type Box = { x: number; y: number; w: number; h: number }
  type Row = { y: number; h: number; start: number; end: number }
  type Head = { y: number; g: Group; shut: boolean }

  function ratioOf(e: Entry) {
    if (e.dir || !isMedia(e)) return 1
    const d = mediaInfo.get(e.path)
    if (!d) return 1.5
    if (!d.w || !d.h) return 1
    return Math.max(0.33, Math.min(3.2, d.w / d.h))
  }

  const layout = $derived.by(() => {
    if (view === 'gallery') media.ver
    const boxes: (Box | undefined)[] = new Array(list.length)
    const rowOf = new Int32Array(list.length).fill(-1)
    const rows: Row[] = []
    const heads: Head[] = []
    const groups = tab.groups ?? [{ key: '', label: '', start: 0, end: list.length }]
    const H = settings.galleryRow
    let y = top0
    const push = (start: number, end: number, h: number) => {
      for (let i = start; i < end; i++) rowOf[i] = rows.length
      rows.push({ y, h, start, end })
    }
    for (const g of groups) {
      if (tab.groups) {
        if (heads.length) y += view === 'list' ? 6 : 10
        const shut = tab.collapsed.has(g.key)
        heads.push({ y, g, shut })
        y += HEAD_H
        if (shut) continue
      }
      if (view === 'list') {
        for (let i = g.start; i < g.end; i++) {
          boxes[i] = { x: pad, y, w: avail, h: LIST_H }
          push(i, i + 1, LIST_H)
          y += LIST_H
        }
      } else if (view === 'grid') {
        for (let i = g.start; i < g.end; i += cols) {
          const end = Math.min(i + cols, g.end)
          for (let j = i; j < end; j++) boxes[j] = { x: pad + (j - i) * (tileW + gap), y, w: tileW, h: tileH }
          push(i, end, tileH)
          y += tileH + gap
        }
      } else {
        let start = g.start
        let sum = 0
        const flush = (end: number, last: boolean) => {
          let h = (avail - gap * (end - start - 1)) / sum
          if (last && h > H) h = H
          h = Math.round(h)
          let x = pad
          for (let i = start; i < end; i++) {
            // Ostatni w pełnym wierszu domyka krawędź co do piksela.
            const w = i === end - 1 && !last ? pad + avail - x : Math.round(ratioOf(list[i]) * h)
            boxes[i] = { x, y, w, h }
            x += w + gap
          }
          push(start, end, h)
          y += h + gap
          start = end
          sum = 0
        }
        for (let i = g.start; i < g.end; i++) {
          sum += ratioOf(list[i])
          if (sum * H + gap * (i - start) >= avail) flush(i + 1, false)
        }
        if (start < g.end) flush(g.end, true)
      }
    }
    return { boxes, rowOf, rows, heads, total: y + pad }
  })

  const totalH = $derived(layout.total)
  const visible = $derived.by(() => {
    const out: { e: Entry; i: number; x: number; y: number; w: number; h: number }[] = []
    const lo = scrollTop - 300, hi = scrollTop + height + 300
    const R = layout.rows
    let a = 0, b = R.length
    while (a < b) {
      const m = (a + b) >> 1
      if (R[m].y + R[m].h < lo) a = m + 1
      else b = m
    }
    for (let r = a; r < R.length && R[r].y <= hi; r++)
      for (let i = R[r].start; i < R[r].end; i++) out.push({ e: list[i], i, ...layout.boxes[i]! })
    return out
  })
  const heads = $derived(layout.heads.filter((h) => h.y + HEAD_H >= scrollTop - 300 && h.y <= scrollTop + height + 300))

  /** Element w sąsiednim wierszu najbliższy w poziomie (działa też przez granice grup). */
  function vertical(f: number, dir: 1 | -1) {
    const r = layout.rowOf[f]
    const t = layout.rows[r + dir]
    if (r < 0 || !t) return f
    const b = layout.boxes[f]!, cx = b.x + b.w / 2
    let best = t.start, bd = Infinity
    for (let i = t.start; i < t.end; i++) {
      const c = layout.boxes[i]!
      const d = Math.abs(c.x + c.w / 2 - cx)
      if (d < bd) {
        bd = d
        best = i
      }
    }
    return best
  }

  function toggleGroup(key: string) {
    if (tab.collapsed.has(key)) tab.collapsed.delete(key)
    else tab.collapsed.add(key)
  }
  function selectGroup(ev: MouseEvent, g: Group) {
    activate(tab.id)
    app.focusedPane = pane
    if (!ev.ctrlKey) tab.selected.clear()
    for (let i = g.start; i < g.end; i++) tab.selected.add(list[i].path)
    tab.focus = g.start
    tab.anchor = g.start
  }

  // Pamięć przewinięcia przy powrocie do folderu i po przełączeniu karty.
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
    if (!tab.search) tab.scrollMemory.set(tab.path.toLowerCase(), scrollTop)
  }

  function ensureVisible(i: number) {
    const b = layout.boxes[i]
    if (!scroller || !b) return
    const head = tab.groups && layout.rows[layout.rowOf[i]]?.start === tab.groups.find((g) => i >= g.start && i < g.end)?.start ? HEAD_H : 0
    if (b.y - head < scroller.scrollTop) scroller.scrollTop = b.y - head - pad
    else if (b.y + b.h > scroller.scrollTop + height) scroller.scrollTop = b.y + b.h - height + pad
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
    for (const row of layout.rows) {
      if (row.y > y1 || row.y + row.h < y0) continue
      for (let i = row.start; i < row.end; i++) {
        const b = layout.boxes[i]!
        const bw = view === 'list' ? Math.min(b.w, 420) : b.w
        if (b.x < x1 && b.x + bw > x0) sel.add(list[i].path)
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
    const pageRows = Math.max(1, Math.floor(height / (view === 'list' ? LIST_H : view === 'grid' ? tileH + gap : settings.galleryRow)) - 1)
    const vert = (d: 1 | -1, times = 1) => {
      if (f < 0) return layout.rows[0]?.start ?? 0
      let to = f
      for (let k = 0; k < times; k++) to = vertical(to, d)
      return to
    }
    // W prawo/lewo omijamy zwinięte grupy.
    const side = (d: 1 | -1) => {
      let to = f + d
      while (to >= 0 && to < n && !layout.boxes[to]) to += d
      return to < 0 || to >= n ? f : to
    }
    switch (ev.key) {
      case 'ArrowDown': return n && move(vert(1))
      case 'ArrowUp': return n && move(vert(-1))
      case 'ArrowRight': if (view !== 'list' && !ev.altKey) return n && move(side(1)); break
      case 'ArrowLeft': if (view !== 'list' && !ev.altKey) return n && move(side(-1)); break
      case 'PageDown': return n && move(vert(1, pageRows))
      case 'PageUp': return n && move(vert(-1, pageRows))
      case 'Home': return n && move(layout.rows[0]?.start ?? 0)
      case 'End': return n && move((layout.rows.at(-1)?.end ?? n) - 1)
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
        else if (tab.selectedEntries.length > 1) app.dialog = { type: 'bulkRename', entries: tab.selectedEntries }
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
  // Przesunięcie dwoma palcami w bok (gładzik): w prawo – wstecz, w lewo – dalej, jak w przeglądarce.
  let swipe = 0
  let swipeAt = 0
  let swipeLock = 0
  function onSwipe(ev: WheelEvent) {
    if (Math.abs(ev.deltaX) <= Math.abs(ev.deltaY) * 1.5 || ev.deltaMode !== 0) return
    const now = performance.now()
    if (now < swipeLock) return
    if (now - swipeAt > 300) swipe = 0
    swipeAt = now
    swipe += ev.deltaX
    if (Math.abs(swipe) > 160) {
      if (swipe < 0) back(tab)
      else forward(tab)
      swipe = 0
      swipeLock = now + 700
    }
  }

  function onWheel(ev: WheelEvent) {
    if (!ev.ctrlKey) return onSwipe(ev)
    ev.preventDefault()
    const d = ev.deltaY < 0 ? 12 : -12
    if (view === 'list') {
      if (d > 0) tab.view = 'grid'
    } else if (view === 'grid') {
      settings.gridSize = Math.max(84, Math.min(280, settings.gridSize + d))
      if (settings.gridSize <= 84 && d < 0) tab.view = 'list'
    } else settings.galleryRow = Math.max(100, Math.min(480, settings.galleryRow + d))
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
    } else if (one && isBrowsableArchive(one)) {
      items.push(
        { label: 'Otwórz jako folder', icon: 'archive', kbd: 'Enter', run: () => navigate(tab, one.path) },
        { label: 'Wypakuj tutaj', icon: 'download', run: () => extractHere(one.path) },
        { label: 'Otwórz w programie', icon: 'open', run: () => api.open(one.path) },
      )
    } else if (one) {
      items.push({ label: 'Otwórz', icon: 'open', kbd: 'Enter', run: () => openEntry(tab, one) })
      items.push({ label: 'Podgląd', icon: 'eye', kbd: 'Spacja', run: () => openViewer(tab, one) })
      items.push({ label: 'Otwórz za pomocą…', icon: 'app', run: () => api.openWith(one.path) })
      if (one.ext === 'lnk') {
        items.push({ label: 'Otwórz lokalizację pliku', icon: 'folder', run: () => openShortcutTarget(tab, one) })
        items.push({ label: 'Otwórz lokalizację w nowej karcie', icon: 'plus', run: () => openShortcutTarget(tab, one, true) })
      }
    }
    if (sel.some((x) => inArchive(x.path))) {
      items.push({ label: 'Wypakuj do…', icon: 'download', run: async () => { const d = await api.pickFolder(); if (d) extractTo(sel.map((x) => x.path), d) } })
      if (other && !other.special) items.push({ label: `Wypakuj do „${other.title}”`, icon: 'split', run: () => extractTo(sel.map((x) => x.path), other.path) })
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
    else items.push({ label: `Zmień nazwy (${sel.length})…`, icon: 'edit', kbd: 'F2', run: () => (app.dialog = { type: 'bulkRename', entries: sel }) })
    if (!sel.some((x) => inArchive(x.path))) {
      items.push({ label: 'Spakuj do ZIP…', icon: 'archive', run: () => zipSelection(tab) })
      if (sel.length > 1) items.push({ label: 'Przenieś do nowego folderu…', icon: 'folderPlus', run: () => newFolderWith(tab) })
    }
    items.push({ label: 'Do Kosza', icon: 'trash', kbd: 'Del', run: () => trash(tab) })
    items.push({ label: 'Usuń trwale', icon: 'trash', kbd: 'Shift+Del', danger: true, run: () => trash(tab, true) })
    items.push({ sep: true })
    if (one?.dir) {
      items.push({ label: 'Przypnij w panelu bocznym', icon: 'pin', run: () => addFavorite(one.path) })
      items.push({ label: isHomeDir(one.path) ? 'Usuń z ekranu Komputer' : 'Pokaż na ekranie Komputer', icon: 'grid', run: () => toggleHomeDir(one.path) })
      items.push({ label: 'Synchronizuj z dyskiem…', icon: 'sync', run: () => (app.dialog = { type: 'sync', profile: null, source: one.path }) })
      items.push({ label: 'Terminal tutaj', icon: 'terminal', run: () => api.terminal(one.path) })
    }
    items.push({ label: 'Dodaj do schowka plików', icon: 'archive', kbd: 'Ctrl+Shift+S', run: () => shelfAdd(sel) })
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
        ...sortMenuItems(tab),
        { label: 'Odśwież', icon: 'refresh', kbd: 'F5', run: () => load(tab, true) },
        { label: 'Zaznacz wszystko', icon: 'check', kbd: 'Ctrl+A', run: () => list.forEach((e) => tab.selected.add(e.path)) },
        { sep: true },
        { label: 'Przypnij ten folder', icon: 'pin', run: () => addFavorite(tab.path) },
        { label: 'Synchronizuj ten folder…', icon: 'sync', run: () => (app.dialog = { type: 'sync', profile: null, source: tab.path }) },
        { label: 'Terminal tutaj', icon: 'terminal', run: () => api.terminal(tab.path) },
        { label: 'Zajętość i porządki', icon: 'grid', run: () => openSpace(tab, tab.path) },
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

  /** Podpowiedź jak w Eksploratorze: typ, rozmiar, daty i – dla multimediów – wymiary i długość. */
  function tip(e: Entry) {
    const m = info(e)
    const lines = [e.name, `Typ: ${typeLabel(e)}`]
    if (e.dir) {
      const d = (dirSizeVer.ver, dirSizes.get(e.path.toLowerCase()))
      if (d) lines.push(`Rozmiar: ${size(d.size)}`, `Zawiera: ${itemsLabel(d.files)}, folderów: ${d.dirs}`)
      lines.push(`Data utworzenia: ${date(e.created)}`)
    } else {
      if (m?.w) lines.push(`Wymiary: ${m.w} × ${m.h}`)
      if (m?.dur) lines.push(`Długość: ${duration(m.dur)}`)
      if (m?.taken && m.taken !== e.modified) lines.push(`Data wykonania: ${date(m.taken)}`)
      lines.push(`Rozmiar: ${size(e.size)}`, `Data modyfikacji: ${date(e.modified)}`)
    }
    if (e.readonly || e.hidden) lines.push(`Atrybuty: ${[e.readonly && 'tylko do odczytu', e.hidden && 'ukryty'].filter(Boolean).join(', ')}`)
    if (e.cloud) lines.push('Dostępny tylko w chmurze')
    if (tab.search) lines.push(`Lokalizacja: ${parent(e.path) ?? ''}`)
    return lines.join('\n')
  }

  function cell(e: Entry, k: ListCol) {
    switch (k) {
      case 'modified': return date(e.modified)
      case 'created': return date(e.created)
      case 'type': return typeLabel(e)
      case 'ext': return e.dir ? '' : e.ext.toUpperCase()
      case 'size': return e.dir ? dsize(e) : size(e.size)
      case 'dims': { const m = info(e); return m?.w ? `${m.w} × ${m.h}` : '' }
      case 'taken': { const m = info(e); return m?.taken ? date(m.taken) : '' }
      case 'duration': { const m = info(e); return m?.dur ? duration(m.dur) : '' }
      case 'attrs': return [e.readonly && 'R', e.hidden && 'H', e.system && 'S', e.link && 'L', e.cloud && 'C'].filter(Boolean).join(' ')
    }
  }

  function colMenu(ev: MouseEvent) {
    ev.preventDefault()
    ev.stopPropagation()
    const all = Object.keys(LIST_COLS) as ListCol[]
    app.menu = {
      x: ev.clientX, y: ev.clientY,
      items: [
        ...all.map((k) => ({
          label: (settings.listCols.includes(k) ? '✓ ' : '   ') + LIST_COLS[k].label,
          run: () => {
            const on = settings.listCols.includes(k)
            settings.listCols = on ? settings.listCols.filter((x) => x !== k) : [...settings.listCols, k]
            if (!on && (k === 'dims' || k === 'taken' || k === 'duration')) ensureMedia(list)
          },
        })),
        { sep: true },
        { label: 'Przywróć domyślne kolumny', icon: 'undo', run: () => { settings.listCols = ['modified', 'type', 'size']; settings.listColW = {} } },
      ],
    }
  }

  // Nagłówki kolumn: klik sortuje, przeciągnięcie zmienia kolejność, krawędź – szerokość.
  const colW = (k: ListCol) => settings.listColW[k] ?? LIST_COLS[k].width
  let headerEl = $state<HTMLDivElement>()
  let colDrag = $state<{ k: ListCol; x0: number; dx: number; moved: boolean } | null>(null)

  function colDown(ev: PointerEvent, k: ListCol) {
    if (ev.button !== 0) return
    ;(ev.currentTarget as HTMLElement).setPointerCapture(ev.pointerId)
    colDrag = { k, x0: ev.clientX, dx: 0, moved: false }
  }
  function colMove(ev: PointerEvent) {
    if (!colDrag) return
    colDrag.dx = ev.clientX - colDrag.x0
    if (Math.abs(colDrag.dx) > 5) colDrag.moved = true
  }
  function colUp(ev: PointerEvent, k: ListCol) {
    const d = colDrag
    colDrag = null
    if (!d) return
    if (!d.moved) {
      const c = LIST_COLS[k]
      if (c.sort) setSort(c.sort)
      return
    }
    const order = settings.listCols.filter((x) => x !== k)
    let idx = order.length
    for (const cell of headerEl?.querySelectorAll<HTMLElement>('[data-col]') ?? []) {
      const ck = cell.dataset.col as ListCol
      if (ck === k) continue
      const r = cell.getBoundingClientRect()
      if (ev.clientX < r.left + r.width / 2) {
        idx = order.indexOf(ck)
        break
      }
    }
    order.splice(idx, 0, k)
    settings.listCols = order
  }
  function gripDown(ev: PointerEvent, k: ListCol) {
    ev.stopPropagation()
    ev.preventDefault()
    const w0 = colW(k), x0 = ev.clientX
    const move = (e: PointerEvent) => (settings.listColW[k] = Math.max(48, Math.min(600, Math.round(w0 + e.clientX - x0))))
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', () => window.removeEventListener('pointermove', move), { once: true })
  }

  const setSort = (k: SortKey) => setSortOf(tab, k)


  const marqueeStyle = $derived(
    marquee
      ? `left:${Math.min(marquee.x0, marquee.x1)}px;top:${Math.min(marquee.y0, marquee.y1)}px;width:${Math.abs(marquee.x1 - marquee.x0)}px;height:${Math.abs(marquee.y1 - marquee.y0)}px`
      : '',
  )
</script>

<div class="wrap">
  {#if view === 'list'}
    <div class="header" role="row" tabindex="-1" oncontextmenu={colMenu} bind:this={headerEl}>
      <button class="c name" onclick={() => setSort('name')}>Nazwa {tab.sort === 'name' ? (tab.desc ? '↓' : '↑') : ''}</button>
      {#if tab.search}<span class="c loc">Lokalizacja</span>{/if}
      {#each settings.listCols as k (k)}
        {@const c = LIST_COLS[k]}
        <span class="hc" data-col={k} class:dragging={colDrag?.k === k && colDrag.moved} style:width="{colW(k)}px" style:transform={colDrag?.k === k && colDrag.moved ? `translateX(${colDrag.dx}px)` : undefined}>
          <button class="c" class:right={c.right} title={c.sort ? 'Sortuj · przeciągnij, aby przenieść' : 'Przeciągnij, aby przenieść'} onpointerdown={(e) => colDown(e, k)} onpointermove={colMove} onpointerup={(e) => colUp(e, k)} oncontextmenu={colMenu}>{c.label} {c.sort && tab.sort === c.sort && (k !== 'ext' || !settings.listCols.includes('type')) ? (tab.desc ? '↓' : '↑') : ''}</button>
          <span class="grip" role="separator" aria-orientation="vertical" onpointerdown={(e) => gripDown(e, k)} ondblclick={() => delete settings.listColW[k]}></span>
        </span>
      {/each}
      <button class="colbtn" title="Kolumny" onclick={colMenu}><Icon name="more" size={14} /></button>
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
  aria-label={tab.search ? `Wyniki wyszukiwania „${tab.search.query}”` : `Zawartość: ${tab.title}`}
  aria-activedescendant={tab.focus >= 0 && visible.some((v) => v.i === tab.focus) ? `it-${pane}-${tab.focus}` : undefined}
>
  <div class="spacer" style:height="{totalH}px"></div>

  {#each heads as h (h.g.key)}
    <div class="ghead" class:list={view === 'list'} style="transform:translate({pad}px,{h.y}px);width:{avail}px">
      <button class="gchev" class:shut={h.shut} title={h.shut ? 'Rozwiń grupę' : 'Zwiń grupę'} onclick={() => toggleGroup(h.g.key)}><Icon name="chevronDown" size={14} /></button>
      <button class="glabel" title="Zaznacz elementy grupy (Ctrl – dodaj)" onclick={(ev) => selectGroup(ev, h.g)}>{h.g.label}<span class="gcount">{h.g.end - h.g.start}</span></button>
      <span class="gline"></span>
    </div>
  {/each}


  {#each visible as { e, i, x, y, w, h } (e.path)}
    {@const sel = tab.selected.has(e.path)}
    <div
      class="item"
      class:sel
      class:focus={tab.focus === i && pane === app.focusedPane}
      class:hidden={e.hidden}
      class:cut={false}
      class:droptarget={dropTarget === e.path}
      style="transform:translate({x}px,{y}px);width:{w}px;height:{h}px"
      role="option"
      id="it-{pane}-{i}"
      tabindex="-1"
      aria-selected={sel}
      aria-label={`${e.name}, ${e.dir ? 'folder' : typeLabel(e) + ', ' + size(e.size)}`}
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
      title={tip(e)}
    >
      {#if view === 'list'}
        <span class="ic" draggable="true"><Thumb entry={e} px={20} iconOnly pad={false} stamp={tab.stamp} /></span>
        {#if tab.renaming === e.path}
          <input class="rename" value={e.name} use:renameMount={e} onkeydown={(ev) => renameKey(ev, e)} onblur={(ev) => doRename(tab, e, ev.currentTarget.value)} onmousedown={(ev) => ev.stopPropagation()} />
        {:else}
          <span class="c name ellipsis" draggable="true"><span class="ellipsis">{e.name}</span>{#if kindOf(e) === 'video' && !settings.listCols.includes('duration') && info(e)?.dur}<span class="durpill">{duration(info(e)!.dur)}</span>{/if}{#if e.cloud}<span class="cloud" title="Tylko w chmurze"><Icon name="cloud" size={13} /></span>{/if}</span>
        {/if}
        {#if tab.search}<span class="c loc ellipsis faint">{parent(e.path)}</span>{/if}
        {#each settings.listCols as k (k)}
          <span class="c ellipsis" class:right={LIST_COLS[k].right} style:width="{colW(k)}px">{cell(e, k)}</span>
        {/each}
      {:else}
        <div class="thumb" class:media={isMedia(e)} class:folder={e.dir}>
          <Thumb entry={e} px={view === 'gallery' ? Math.max(w, h) : tileW} fit={view === 'gallery' ? 'cover' : 'contain'} iconOnly={e.dir} stamp={tab.stamp} />
          {#if kindOf(e) === 'video'}{@const m = info(e)}<span class="badge" class:dur={!!m?.dur}><Icon name="play" size={10} fill />{#if m?.dur}<span>{duration(m.dur)}</span>{/if}</span>{/if}
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
      {:else if tab.filter || tab.kindFilter !== 'all' || tab.facets.size}
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
    background: var(--hover);
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
    border-radius: var(--r);
    will-change: transform;
    contain: layout style;
  }
  /* Płynne przejścia przy zmianie rozmiaru, widoku, sortowania i grup. */
  .item,
  .ghead {
    transition:
      transform 0.22s var(--ease),
      width 0.22s var(--ease),
      height 0.22s var(--ease);
  }
  @media (prefers-reduced-motion: reduce) {
    .item,
    .ghead {
      transition: none;
    }
  }
  .ghead {
    position: absolute;
    left: 0;
    top: 0;
    height: 38px;
    display: flex;
    align-items: center;
    gap: 4px;
    z-index: 1;
  }
  .gchev {
    width: 24px;
    height: 24px;
    border-radius: var(--r-xs);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--faint);
    transition: transform 0.15s var(--ease);
  }
  .gchev.shut {
    transform: rotate(-90deg);
  }
  .gchev:hover,
  .glabel:hover {
    background: var(--hover);
    color: var(--text);
  }
  .glabel {
    display: flex;
    align-items: baseline;
    gap: 8px;
    height: 26px;
    padding: 0 8px;
    border-radius: var(--r-xs);
    font-weight: 600;
    font-size: 13px;
    color: var(--text);
  }
  .gcount {
    font-weight: 500;
    font-size: 11px;
    color: var(--ghost);
  }
  .gline {
    flex: 1;
    height: 1px;
    background: var(--line);
    margin-left: 6px;
  }
  .durpill {
    flex: none;
    font-size: 11px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--high);
    color: var(--dim);
    font-variant-numeric: tabular-nums;
  }
  .hc {
    position: relative;
    flex: none;
    display: flex;
  }
  .hc .c {
    width: 100%;
    cursor: default;
  }
  .hc.dragging {
    z-index: 3;
    background: var(--high);
    border-radius: var(--r-xs);
    opacity: 0.9;
  }
  .grip {
    position: absolute;
    right: -3px;
    top: 4px;
    bottom: 4px;
    width: 6px;
    cursor: col-resize;
    z-index: 2;
  }
  .grip::after {
    content: '';
    position: absolute;
    left: 2px;
    top: 3px;
    bottom: 3px;
    width: 1px;
    background: var(--line);
  }
  .header:hover .grip::after {
    background: var(--line-strong);
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
    position: relative;
    background: var(--bg);
    color: var(--faint);
    font-size: 12px;
  }
  .header .c {
    text-align: left;
    padding: 4px 6px;
    border-radius: var(--r-xs);
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
    box-shadow: inset 0 0 0 1px var(--accent);
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
  .c.right {
    text-align: right;
  }
  .colbtn {
    width: 22px;
    height: 22px;
    position: absolute;
    right: 0;
    border-radius: var(--r-xs);
    color: var(--ghost);
    display: flex;
    align-items: center;
    justify-content: center;
    flex: none;
  }
  .colbtn:hover {
    background: var(--hover);
    color: var(--text);
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
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .grid .thumb {
    position: relative;
    width: 100%;
    flex: 1;
    min-height: 0;
    border-radius: var(--r-s);
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
    border-radius: var(--r-xs);
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
    border-radius: var(--r-s);
    overflow: hidden;
  }
  .gallery .item.sel {
    background: var(--sel);
  }
  .gallery .item.focus {
    box-shadow: inset 0 0 0 2px var(--line-strong);
  }
  .gallery .item.droptarget {
    box-shadow: inset 0 0 0 2px var(--accent);
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
    background: var(--accent);
    color: var(--on-accent);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .badge {
    position: absolute;
    right: 6px;
    bottom: 6px;
    min-width: 22px;
    height: 22px;
    gap: 4px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    background: rgba(0, 0, 0, 0.6);
    color: #fff;
    display: flex;
    align-items: center;
    justify-content: center;
    padding-left: 2px;
  }
  .badge.dur {
    padding: 0 8px 0 7px;
  }

  .marquee {
    position: absolute;
    z-index: 4;
    background: var(--hover);
    border: 1px solid var(--ghost);
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
    background: var(--accent);
    color: var(--on-accent);
    border-radius: 999px;
    font: 600 12px var(--font);
  }
</style>
