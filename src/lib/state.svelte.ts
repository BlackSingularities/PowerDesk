import { SvelteSet } from 'svelte/reactivity'
import { api, on, type ConflictInfo, type ConflictPolicy, type Drive, type Entry, type Known, type OpProgress, type SyncProfile, type SyncStatus } from './api'
import { basename, collator, debounce, join, kindOf, isMedia, parent, samePath, volumeOf, type Kind } from './util'

export type View = 'list' | 'grid' | 'gallery'
export type SortKey = 'name' | 'modified' | 'size' | 'type' | 'created'
export type KindFilter = 'all' | 'media' | 'image' | 'video' | 'doc' | 'audio' | 'archive'
export interface FolderPrefs { view?: View; sort?: SortKey; desc?: boolean }

let seq = 0
const uid = () => `t${Date.now().toString(36)}${(seq++).toString(36)}`

export const HOME = 'ofp:home'
export const SYNC = 'ofp:sync'
export const SETTINGS = 'ofp:settings'

export function specialTitle(p: string) {
  if (p === HOME) return 'Ten komputer'
  if (p === SYNC) return 'Synchronizacja'
  if (p === SETTINGS) return 'Ustawienia'
  return basename(p)
}

const KIND_GROUPS: Record<Exclude<KindFilter, 'all'>, Kind[]> = {
  media: ['image', 'raw', 'video'],
  image: ['image', 'raw'],
  video: ['video'],
  doc: ['pdf', 'doc', 'sheet', 'slides', 'text'],
  audio: ['audio'],
  archive: ['archive'],
}

export class Tab {
  id = uid()
  path = $state(HOME)
  history: string[] = $state([HOME])
  hIndex = $state(0)
  entries: Entry[] = $state.raw([])
  loading = $state(false)
  error = $state('')
  selected = new SvelteSet<string>()
  focus = $state(-1)
  anchor = -1
  filter = $state('')
  kindFilter = $state<KindFilter>('all')
  view = $state<View>('list')
  sort = $state<SortKey>('name')
  desc = $state(false)
  renaming = $state<string | null>(null)
  search = $state<{ query: string; id: number; running: boolean; scanned: number; results: Entry[] } | null>(null)
  scrollTop = 0
  scrollMemory = new Map<string, number>()
  /** Zmienia się po każdym przeładowaniu, żeby widok mógł odświeżyć miniatury. */
  stamp = $state(0)

  source = $derived(this.search ? this.search.results : this.entries)

  items: Entry[] = $derived.by(() => {
    const hidden = settings.showHidden
    const f = this.filter.trim().toLowerCase()
    const kf = this.kindFilter
    const group = kf === 'all' ? null : KIND_GROUPS[kf]
    const arr = this.source.filter(
      (e) =>
        (hidden || !(e.hidden || e.system)) &&
        (!f || e.name.toLowerCase().includes(f)) &&
        (!group || (!e.dir && group.includes(kindOf(e)))),
    )
    const dir = this.desc ? -1 : 1
    const key = this.sort
    const cmp = (a: Entry, b: Entry) => {
      let r = 0
      if (key === 'modified') r = a.modified - b.modified
      else if (key === 'created') r = a.created - b.created
      else if (key === 'size') r = a.size - b.size
      else if (key === 'type') r = collator.compare(a.ext, b.ext)
      if (r === 0) r = collator.compare(a.name, b.name)
      return r * dir
    }
    arr.sort((a, b) => (settings.foldersFirst && a.dir !== b.dir ? (a.dir ? -1 : 1) : cmp(a, b)))
    return arr
  })

  selectedEntries = $derived(this.items.filter((e) => this.selected.has(e.path)))
  get special() {
    return this.path.startsWith('ofp:')
  }
  title = $derived(this.search ? `„${this.search.query}”` : specialTitle(this.path))

  constructor(path = HOME) {
    this.path = path
    this.history = [path]
  }
}

// ---------- Ustawienia ----------

export const settings = $state({
  showHidden: false,
  foldersFirst: true,
  confirmTrash: false,
  closeToTray: true,
  previewPane: true,
  sidebarWidth: 248,
  previewWidth: 360,
  gridSize: 132,
  galleryRow: 210,
  autoGallery: true,
  pdfThumbs: true,
  singleClickOpen: false,
  defaultView: 'list' as View,
  rememberViewPerFolder: true,
  checkUpdates: true,
  playVideoPreview: true,
  collapsed: {} as Record<string, boolean>,
})

export const app = $state({
  tabs: [] as Tab[],
  panes: [] as string[],
  focusedPane: 0,
  split: false,
  drives: [] as Drive[],
  known: [] as Known[],
  favorites: [] as { name: string; path: string }[],
  recent: [] as string[],
  folderPrefs: {} as Record<string, FolderPrefs>,
  closed: [] as string[],
  ops: [] as OpProgress[],
  toasts: [] as { id: number; text: string; kind: 'info' | 'ok' | 'error'; action?: { label: string; run: () => void } }[],
  syncProfiles: [] as SyncProfile[],
  syncStatus: {} as Record<string, SyncStatus>,
  viewer: null as { list: Entry[]; index: number } | null,
  dialog: null as Dialog | null,
  menu: null as { x: number; y: number; items: MenuItem[] } | null,
  palette: false,
  sidebarOpen: true,
  ready: false,
  update: null as { latest: string; url: string } | null,
  dragging: null as { paths: string[] } | null,
})

export type MenuItem =
  | { label: string; icon?: string; kbd?: string; run: () => void; danger?: boolean; disabled?: boolean }
  | { sep: true }
  | { label: string; icon?: string; sub: MenuItem[] }

export type Dialog =
  | { type: 'prompt'; title: string; label?: string; value: string; selectStem?: boolean; ok: string; resolve: (v: string | null) => void }
  | { type: 'confirm'; title: string; text: string; ok: string; danger?: boolean; resolve: (v: boolean) => void }
  | { type: 'conflict'; conflicts: ConflictInfo[]; mode: 'copy' | 'move'; resolve: (v: ConflictPolicy | null) => void }
  | { type: 'props'; entries: Entry[] }
  | { type: 'network' }
  | { type: 'sync'; profile: SyncProfile | null; source?: string }
  | { type: 'syncPreview'; id: string }

export function prompt(title: string, value = '', ok = 'OK', selectStem = false, label?: string) {
  return new Promise<string | null>((resolve) => (app.dialog = { type: 'prompt', title, value, ok, selectStem, label, resolve }))
}
export function confirm(title: string, text: string, ok = 'OK', danger = false) {
  return new Promise<boolean>((resolve) => (app.dialog = { type: 'confirm', title, text, ok, danger, resolve }))
}

let toastSeq = 0
export function toast(text: string, kind: 'info' | 'ok' | 'error' = 'info', action?: { label: string; run: () => void }, ms = 3800) {
  const id = ++toastSeq
  app.toasts = [...app.toasts.slice(-3), { id, text, kind, action }]
  setTimeout(() => (app.toasts = app.toasts.filter((t) => t.id !== id)), action ? ms + 2500 : ms)
}

// ---------- Karty ----------

export function tabById(id: string) {
  return app.tabs.find((t) => t.id === id)
}
export function activeTab(): Tab {
  return tabById(app.panes[app.focusedPane]) ?? app.tabs[0]
}

export function newTab(path = HOME, activate = true, afterId?: string) {
  const t = new Tab(path)
  applyPrefs(t)
  const at = afterId ? app.tabs.findIndex((x) => x.id === afterId) + 1 : app.tabs.length
  app.tabs.splice(at, 0, t)
  if (activate) app.panes[app.focusedPane] = t.id
  load(t)
  return t
}

export function activate(id: string) {
  const other = app.split ? 1 - app.focusedPane : -1
  if (other >= 0 && app.panes[other] === id) {
    app.focusedPane = other
    return
  }
  app.panes[app.focusedPane] = id
}

export function closeTab(id: string) {
  const i = app.tabs.findIndex((t) => t.id === id)
  if (i < 0) return
  const t = app.tabs[i]
  if (!t.special) app.closed = [t.path, ...app.closed].slice(0, 20)
  app.tabs.splice(i, 1)
  if (!app.tabs.length) newTab(HOME)
  for (let p = 0; p < app.panes.length; p++) {
    if (app.panes[p] === id) {
      const fallback = app.tabs[Math.min(i, app.tabs.length - 1)]
      const otherId = app.panes[1 - p]
      const pick = fallback.id === otherId ? app.tabs.find((x) => x.id !== otherId) : fallback
      if (pick) app.panes[p] = pick.id
      else {
        app.split = false
        app.panes = [otherId]
        app.focusedPane = 0
      }
    }
  }
}

export function reopenClosed() {
  const p = app.closed.shift()
  if (p) newTab(p)
}

export function toggleSplit() {
  if (app.split) {
    app.panes = [app.panes[app.focusedPane]]
    app.focusedPane = 0
    app.split = false
    return
  }
  const cur = activeTab()
  const other = app.tabs.find((t) => t.id !== cur.id) ?? newTab(cur.path, false, cur.id)
  app.panes = [cur.id, other.id]
  app.split = true
  app.focusedPane = 0
}

// ---------- Nawigacja ----------

function applyPrefs(t: Tab) {
  const p = settings.rememberViewPerFolder ? app.folderPrefs[t.path.toLowerCase()] : undefined
  t.view = p?.view ?? settings.defaultView
  t.sort = p?.sort ?? 'name'
  t.desc = p?.desc ?? false
}

export function savePrefs(t: Tab) {
  if (t.special) return
  app.folderPrefs[t.path.toLowerCase()] = { view: t.view, sort: t.sort, desc: t.desc }
}

export async function load(t: Tab, keepSelection = false) {
  if (t.special) {
    t.entries = []
    t.error = ''
    if (t.path === HOME) refreshDrives()
    return
  }
  const path = t.path
  t.loading = true
  try {
    const list = await api.listDir(path)
    if (t.path !== path) return
    t.entries = list
    t.error = ''
    t.stamp++
    if (keepSelection) {
      const alive = new Set(list.map((e) => e.path))
      for (const s of [...t.selected]) if (!alive.has(s)) t.selected.delete(s)
    }
    // Folder pełen zdjęć/filmów od razu otwiera się w galerii (o ile nie ustawiono inaczej).
    if (!keepSelection && settings.autoGallery && !app.folderPrefs[path.toLowerCase()] && list.length >= 6) {
      const files = list.filter((e) => !e.dir)
      const media = files.filter(isMedia).length
      if (files.length && media / files.length >= 0.7) t.view = 'gallery'
    }
  } catch (e) {
    if (t.path !== path) return
    t.entries = []
    t.error = String(e)
  } finally {
    if (t.path === path) t.loading = false
  }
}

export function navigate(t: Tab, path: string, push = true) {
  if (!path) return
  if (/^[a-zA-Z]:$/.test(path)) path += '\\'
  if (!t.special) t.scrollMemory.set(t.path.toLowerCase(), t.scrollTop)
  const prev = t.path
  t.path = path
  t.search = null
  t.filter = ''
  t.kindFilter = 'all'
  t.renaming = null
  t.selected.clear()
  t.focus = -1
  t.anchor = -1
  t.entries = []
  if (push) {
    t.history = [...t.history.slice(0, t.hIndex + 1), path]
    t.hIndex = t.history.length - 1
  }
  applyPrefs(t)
  if (!path.startsWith('ofp:')) {
    app.recent = [path, ...app.recent.filter((r) => !samePath(r, path))].slice(0, 40)
  }
  // Po wejściu „w górę" zaznaczamy folder, z którego przyszliśmy – jak w Eksploratorze.
  const cameFrom = prev && parent(prev) && samePath(parent(prev)!, path) ? prev : null
  load(t).then(() => {
    if (cameFrom) {
      const i = t.items.findIndex((e) => samePath(e.path, cameFrom))
      if (i >= 0) {
        t.selected.add(t.items[i].path)
        t.focus = i
        t.anchor = i
      }
    }
  })
}

export function back(t: Tab) {
  if (t.search) return closeSearch(t)
  if (t.hIndex > 0) {
    t.hIndex--
    navigate(t, t.history[t.hIndex], false)
  }
}
export function forward(t: Tab) {
  if (t.hIndex < t.history.length - 1) {
    t.hIndex++
    navigate(t, t.history[t.hIndex], false)
  }
}
export function up(t: Tab) {
  const p = parent(t.path)
  if (p) navigate(t, p)
  else if (!t.special) navigate(t, HOME)
}

export function openEntry(t: Tab, e: Entry, newTabToo = false) {
  if (e.dir) {
    if (newTabToo) newTab(e.path, false, t.id)
    else navigate(t, e.path)
    return
  }
  const k = kindOf(e)
  if (k === 'image' || k === 'raw' || k === 'video') {
    const list = t.items.filter((x) => isMedia(x))
    app.viewer = { list, index: Math.max(0, list.findIndex((x) => x.path === e.path)) }
    return
  }
  api.open(e.path).catch((err) => toast(String(err), 'error'))
}

export function openViewer(t: Tab, e: Entry) {
  const list = t.items.filter((x) => !x.dir)
  app.viewer = { list, index: Math.max(0, list.findIndex((x) => x.path === e.path)) }
}

// ---------- Wyszukiwanie ----------

let unlistenSearch: (() => void) | null = null
export async function startSearch(t: Tab, query: string) {
  query = query.trim()
  if (!query || t.special) return
  const id = await api.search(t.path, query, settings.showHidden)
  t.search = { query, id, running: true, scanned: 0, results: [] }
  t.selected.clear()
  t.focus = -1
}
export function closeSearch(t: Tab) {
  api.cancelSearch()
  t.search = null
  t.selected.clear()
}

// ---------- Operacje na plikach ----------

export async function transfer(sources: string[], dest: string, mode: 'copy' | 'move') {
  sources = sources.filter((s) => !samePath(parent(s) ?? '', dest) || mode === 'copy')
  if (!sources.length || dest.startsWith('ofp:')) return
  if (sources.some((s) => samePath(s, dest) || dest.toLowerCase().startsWith(s.toLowerCase() + '\\'))) {
    toast('Nie można przenieść folderu do jego własnego wnętrza', 'error')
    return
  }
  let policy: ConflictPolicy = 'rename'
  const conflicts = (await api.conflicts(sources, dest)).filter((c) => !samePath(c.src, c.dst))
  if (conflicts.length) {
    const res = await new Promise<ConflictPolicy | null>((resolve) => (app.dialog = { type: 'conflict', conflicts, mode, resolve }))
    if (!res) return
    policy = res
  }
  await api.startOp(mode, sources, dest, policy)
}

export const clip = $state({ paths: [] as string[], cut: false })

export async function copySel(t: Tab, cut: boolean) {
  const paths = t.selectedEntries.map((e) => e.path)
  if (!paths.length) return
  clip.paths = paths
  clip.cut = cut
  await api.clipSet(paths, cut).catch(() => {})
  toast(`${cut ? 'Wycięto' : 'Skopiowano'}: ${paths.length === 1 ? basename(paths[0]) : paths.length + ' elem.'}`)
}

export async function paste(t: Tab, into?: string) {
  const dest = into ?? t.path
  if (dest.startsWith('ofp:')) return
  // Systemowy schowek ma pierwszeństwo – działa z plikami skopiowanymi w Eksploratorze.
  const sys = await api.clipGet().catch(() => ({ paths: [] as string[], cut: false }))
  const paths = sys.paths.length ? sys.paths : clip.paths
  const cut = sys.paths.length ? sys.cut : clip.cut
  if (!paths.length) return toast('Schowek jest pusty')
  await transfer(paths, dest, cut ? 'move' : 'copy')
  if (cut) {
    clip.paths = []
    api.clipSet([], false).catch(() => {})
  }
}

export async function trash(t: Tab, permanent = false) {
  const sel = t.selectedEntries
  if (!sel.length) return
  const what = sel.length === 1 ? `„${sel[0].name}”` : `${sel.length} elementów`
  const network = sel.some((e) => e.path.startsWith('\\\\') || app.drives.find((d) => d.kind === 'network' && volumeOf(d.path) === volumeOf(e.path)))
  const removable = sel.some((e) => app.drives.find((d) => d.kind === 'removable' && volumeOf(d.path) === volumeOf(e.path)))
  if (permanent || network || removable) {
    const note = !permanent ? '\nTen dysk nie ma Kosza – usunięcie będzie trwałe.' : ''
    if (!(await confirm('Usunąć trwale?', `${what} zostanie usunięte bez możliwości przywrócenia.${note}`, 'Usuń trwale', true))) return
    await api.startOp('delete', sel.map((e) => e.path))
  } else {
    if (settings.confirmTrash && !(await confirm('Przenieść do Kosza?', `${what} trafi do Kosza.`, 'Do Kosza'))) return
    await api.startOp('trash', sel.map((e) => e.path))
  }
  t.selected.clear()
}

export async function newFolder(t: Tab) {
  if (t.special) return
  const name = await prompt('Nowy folder', 'Nowy folder', 'Utwórz')
  if (!name) return
  try {
    const p = await api.mkdir(t.path, name)
    await load(t)
    selectOnly(t, p)
  } catch (e) {
    toast(String(e), 'error')
  }
}

export async function newFile(t: Tab) {
  if (t.special) return
  const name = await prompt('Nowy plik', 'Nowy dokument tekstowy.txt', 'Utwórz', true)
  if (!name) return
  try {
    const p = await api.mkfile(t.path, name)
    await load(t)
    selectOnly(t, p)
  } catch (e) {
    toast(String(e), 'error')
  }
}

export function selectOnly(t: Tab, path: string) {
  t.selected.clear()
  t.selected.add(path)
  const i = t.items.findIndex((e) => samePath(e.path, path))
  t.focus = i
  t.anchor = i
}

export async function doRename(t: Tab, e: Entry, name: string) {
  t.renaming = null
  if (!name || name === e.name) return
  try {
    const p = await api.rename(e.path, name)
    await load(t, true)
    selectOnly(t, p)
  } catch (err) {
    toast(String(err), 'error')
  }
}

export async function undo() {
  try {
    toast(await api.undo(), 'ok')
  } catch (e) {
    toast(String(e))
  }
}

export function dropMode(sources: string[], dest: string, ev: { ctrlKey: boolean; shiftKey: boolean }): 'copy' | 'move' {
  if (ev.ctrlKey) return 'copy'
  if (ev.shiftKey) return 'move'
  return sources.every((s) => volumeOf(s) === volumeOf(dest)) ? 'move' : 'copy'
}

export function addFavorite(path: string) {
  if (app.favorites.some((f) => samePath(f.path, path))) return
  app.favorites = [...app.favorites, { name: basename(path), path }]
  toast(`Przypięto „${basename(path)}”`, 'ok')
}
export function removeFavorite(path: string) {
  app.favorites = app.favorites.filter((f) => !samePath(f.path, path))
}

// ---------- Dyski i synchronizacja ----------

let drivesBusy = false
export async function refreshDrives() {
  if (drivesBusy) return
  drivesBusy = true
  try {
    app.drives = await api.drives()
  } finally {
    drivesBusy = false
  }
}

export async function refreshSync() {
  const r = await api.syncList()
  app.syncProfiles = r.profiles
  const st: Record<string, SyncStatus> = {}
  for (const s of r.status) st[s.id] = s
  app.syncStatus = st
  api.closeToTray(settings.closeToTray)
}

// ---------- Trwałość ----------

const persist = debounce(() => {
  const data = {
    settings: $state.snapshot(settings),
    favorites: app.favorites,
    recent: app.recent,
    folderPrefs: app.folderPrefs,
    tabs: app.tabs.filter((t) => !t.search).map((t) => t.path),
    active: Math.max(0, app.tabs.findIndex((t) => t.id === app.panes[app.focusedPane])),
    split: app.split,
    sidebarOpen: app.sidebarOpen,
  }
  api.saveState(JSON.stringify(data))
}, 600)

export function persistNow() {
  persist()
}

export async function init() {
  try {
    const s = JSON.parse(await api.loadState())
    if (s.settings) Object.assign(settings, s.settings)
    app.favorites = s.favorites ?? []
    app.recent = s.recent ?? []
    app.folderPrefs = s.folderPrefs ?? {}
    app.sidebarOpen = s.sidebarOpen ?? true
    const paths: string[] = s.tabs?.length ? s.tabs : [HOME]
    for (const p of paths) newTab(p, false)
    const act = app.tabs[Math.min(s.active ?? 0, app.tabs.length - 1)]
    app.panes = [act.id]
    if (s.split && app.tabs.length > 1) {
      app.panes = [act.id, app.tabs.find((t) => t.id !== act.id)!.id]
      app.split = true
    }
  } catch {
    const t = newTab(HOME, false)
    app.panes = [t.id]
  }
  api.known().then((k) => (app.known = k))
  refreshDrives()
  refreshSync()

  on<string[]>('fs-changed', (dirs) => {
    for (const t of app.tabs) {
      if (!t.special && !t.search && dirs.some((d) => samePath(d, t.path))) load(t, true)
    }
  })
  on<OpProgress>('op-progress', (p) => {
    const i = app.ops.findIndex((o) => o.id === p.id)
    if (i >= 0) app.ops[i] = p
    else app.ops = [...app.ops, p]
    if (p.state !== 'running') {
      if (p.state === 'done') toast(doneText(p), 'ok', p.kind === 'copy' || p.kind === 'move' ? { label: 'Cofnij', run: undo } : undefined)
      else if (p.state === 'error') toast(`${p.title}: ${p.errors.length} błędów`, 'error')
      setTimeout(() => (app.ops = app.ops.filter((o) => o.id !== p.id || o.state === 'running')), p.state === 'error' ? 12000 : 1500)
      refreshDrivesSoon()
    }
  })
  on<import('./api').SearchBatch>('search', (b) => {
    for (const t of app.tabs) {
      if (t.search && t.search.id === b.id) {
        t.search.results = [...t.search.results, ...b.items]
        t.search.running = !b.done
        t.search.scanned = b.scanned
      }
    }
  })
  on<SyncStatus>('sync-status', (s) => {
    app.syncStatus[s.id] = s
  })
  on<string>('open-path', (p) => {
    api.stat(p).then((e) => newTab(e.dir ? e.path : parent(e.path) ?? HOME)).catch(() => {})
  })

  // Podłączenie pendrive’a/dysku – lista odświeża się sama.
  setInterval(() => {
    if (!document.hidden) refreshDrives()
  }, 5000)

  $effect.root(() => {
    $effect(() => {
      // Obserwujemy foldery wszystkich otwartych kart.
      const dirs = [...new Set(app.tabs.filter((t) => !t.special).map((t) => t.path))]
      api.watch(dirs)
    })
    $effect(() => {
      JSON.stringify(settings)
      app.favorites.length
      app.recent[0]
      app.tabs.map((t) => t.path).join('|')
      app.panes.join('|')
      app.split
      app.sidebarOpen
      Object.keys(app.folderPrefs).length
      persist()
    })
    $effect(() => {
      api.closeToTray(settings.closeToTray)
    })
  })

  app.ready = true
  if (settings.checkUpdates) {
    setTimeout(() => {
      api.checkUpdate().then((u) => {
        if (u.newer) app.update = { latest: u.latest, url: u.url }
      }).catch(() => {})
    }, 4000)
  }
}

const refreshDrivesSoon = debounce(refreshDrives, 400)

function doneText(p: OpProgress) {
  const n = p.total_items
  switch (p.kind) {
    case 'copy': return `Skopiowano do „${basename(p.dest)}”`
    case 'move': return `Przeniesiono do „${basename(p.dest)}”`
    case 'trash': return `Przeniesiono do Kosza (${n})`
    default: return `Usunięto (${n})`
  }
}

export { join }
