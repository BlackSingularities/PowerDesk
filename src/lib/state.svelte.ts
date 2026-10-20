import { SvelteSet } from 'svelte/reactivity'
import { getCurrentWindow, Effect } from '@tauri-apps/api/window'
import { facetsFor, facetTest, type Facet } from './facets'
import { parseQuery } from './query'
import { api, on, type DirSize, type ConflictInfo, type ConflictPolicy, type Drive, type Entry, type Known, type MediaInfo, type OpProgress, type SyncProfile, type SyncStatus } from './api'
import { basename, collator, debounce, join, kindOf, isMedia, parent, samePath, typeLabel, volumeOf, inArchive, isBrowsableArchive, isRemote, type Kind } from './util'

export type View = 'list' | 'grid' | 'gallery'
export type SortKey = 'name' | 'modified' | 'size' | 'type' | 'created' | 'taken' | 'duration' | 'dims'
export type ListCol = 'modified' | 'created' | 'type' | 'ext' | 'size' | 'dims' | 'taken' | 'duration' | 'attrs'
/** Kolumny widoku listy: etykieta, domyślna szerokość, klucz sortowania. */
export const LIST_COLS: Record<ListCol, { label: string; width: number; sort?: SortKey; right?: boolean }> = {
  modified: { label: 'Zmodyfikowano', width: 140, sort: 'modified' },
  created: { label: 'Utworzono', width: 140, sort: 'created' },
  type: { label: 'Typ', width: 130, sort: 'type' },
  ext: { label: 'Rozszerzenie', width: 90, sort: 'type' },
  size: { label: 'Rozmiar', width: 86, sort: 'size', right: true },
  dims: { label: 'Wymiary', width: 100, sort: 'dims', right: true },
  taken: { label: 'Data wykonania', width: 140, sort: 'taken' },
  duration: { label: 'Czas trwania', width: 90, sort: 'duration', right: true },
  attrs: { label: 'Atrybuty', width: 74 },
}
export const SORT_LABEL: Record<SortKey, string> = {
  name: 'Nazwa', modified: 'Data modyfikacji', created: 'Data utworzenia', size: 'Rozmiar', type: 'Typ',
  taken: 'Data wykonania', duration: 'Czas trwania', dims: 'Wymiary',
}

// ---------- Metadane multimediów (wymiary, czas trwania, data wykonania) ----------

/** Wspólne dla wszystkich kart; `media.ver` rośnie po każdej partii, żeby widoki się przeliczyły. */
export const mediaInfo = new Map<string, MediaInfo>()
export const media = $state({ ver: 0 })
const mediaQueue: Entry[] = []
const mediaQueued = new Set<string>()
let mediaBusy = false
const wantsMedia = (e: Entry) => !e.dir && !e.cloud && !isRemote(e.path) && !inArchive(e.path) && (isMedia(e) || kindOf(e) === 'audio')

/** Pobiera metadane w kolejności listy – nowsze żądanie (bieżący folder) idzie na początek kolejki. */
export function ensureMedia(list: Entry[]) {
  const fresh = list.filter((e) => wantsMedia(e) && !mediaInfo.has(e.path) && !mediaQueued.has(e.path))
  if (!fresh.length) return
  fresh.forEach((e) => mediaQueued.add(e.path))
  mediaQueue.unshift(...fresh)
  if (!mediaBusy) pumpMedia()
}
async function pumpMedia() {
  mediaBusy = true
  while (mediaQueue.length) {
    const part = mediaQueue.splice(0, 48)
    try {
      const res = await api.mediaInfo(part.map((e) => [e.path, e.modified] as [string, number]))
      part.forEach((e, j) => mediaInfo.set(e.path, res[j]))
    } catch {
      part.forEach((e) => mediaInfo.set(e.path, { w: 0, h: 0, dur: 0, taken: 0 }))
    }
    part.forEach((e) => mediaQueued.delete(e.path))
    media.ver++
  }
  mediaBusy = false
}

// ---------- Rozmiary folderów (liczone w tle) ----------

/** Klucz: ścieżka małymi literami. Liczymy po dwa foldery naraz, żeby nie zatkać dysku. */
export const dirSizes = new Map<string, DirSize>()
export const dirSizeVer = $state({ ver: 0 })
const dsQueue: string[] = []
const dsQueued = new Set<string>()
let dsRunning = 0
const isNetworkPath = (p: string) => p.startsWith('\\\\') || isRemote(p) || inArchive(p) || app.drives.some((d) => d.kind === 'network' && volumeOf(d.path) === volumeOf(p))

export function ensureDirSizes(list: Entry[]) {
  if (!settings.folderSizes) return
  const fresh = list.filter((e) => e.dir && !dirSizes.has(e.path.toLowerCase()) && !dsQueued.has(e.path.toLowerCase()) && !isNetworkPath(e.path))
  if (!fresh.length) return
  // Nowsze żądanie (bieżący folder) wyprzedza stare.
  fresh.forEach((e) => dsQueued.add(e.path.toLowerCase()))
  dsQueue.unshift(...fresh.map((e) => e.path))
  while (dsRunning < 2 && dsQueue.length) pumpDirSize()
}
async function pumpDirSize() {
  dsRunning++
  while (dsQueue.length) {
    const p = dsQueue.shift()!
    try {
      dirSizes.set(p.toLowerCase(), await api.dirSize(p))
    } catch {
      dirSizes.set(p.toLowerCase(), { size: 0, files: 0, dirs: 0 })
    }
    dsQueued.delete(p.toLowerCase())
    dirSizeVer.ver++
  }
  dsRunning--
}
/** Rozmiar elementu – dla folderów z tła (0, dopóki nie policzony). */
export const sizeOf = (e: Entry) => (e.dir ? dirSizes.get(e.path.toLowerCase())?.size ?? -1 : e.size)

// ---------- Grupowanie (jak w Eksploratorze) ----------

export type GroupKey = 'none' | 'name' | 'modified' | 'created' | 'taken' | 'type' | 'kind' | 'size'
export const GROUP_LABEL: Record<GroupKey, string> = {
  none: 'Brak', name: 'Nazwa', modified: 'Data modyfikacji', created: 'Data utworzenia', taken: 'Data wykonania',
  type: 'Typ', kind: 'Rodzaj', size: 'Rozmiar',
}
export interface Group { key: string; label: string; start: number; end: number }

const DAY = 86_400_000
const MONTHS = ['Styczeń', 'Luty', 'Marzec', 'Kwiecień', 'Maj', 'Czerwiec', 'Lipiec', 'Sierpień', 'Wrzesień', 'Październik', 'Listopad', 'Grudzień']
/** Przedziały dat jak w Eksploratorze; rank – im mniejszy, tym nowszy. */
function dateGroup(t: number): [number, string] {
  if (!t) return [99999, 'Brak daty']
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  const weekStart = today - ((now.getDay() + 6) % 7) * DAY
  const monthStart = new Date(now.getFullYear(), now.getMonth(), 1).getTime()
  const lastMonth = new Date(now.getFullYear(), now.getMonth() - 1, 1).getTime()
  const yearStart = new Date(now.getFullYear(), 0, 1).getTime()
  if (t >= today + DAY) return [0, 'W przyszłości']
  if (t >= today) return [1, 'Dziś']
  if (t >= today - DAY) return [2, 'Wczoraj']
  if (t >= weekStart) return [3, 'Wcześniej w tym tygodniu']
  if (t >= weekStart - 7 * DAY) return [4, 'W zeszłym tygodniu']
  if (t >= monthStart) return [5, 'Wcześniej w tym miesiącu']
  if (t >= lastMonth) return [6, 'W zeszłym miesiącu']
  const d = new Date(t)
  if (t >= yearStart) return [10 + (11 - d.getMonth()), `${MONTHS[d.getMonth()]} ${d.getFullYear()}`]
  return [100 + (now.getFullYear() - d.getFullYear()), String(d.getFullYear())]
}
const KIND_LABEL: [Kind[], string][] = [
  [['folder'], 'Foldery'], [['image', 'raw'], 'Zdjęcia'], [['video'], 'Wideo'], [['audio'], 'Muzyka'],
  [['pdf', 'doc', 'sheet', 'slides', 'text'], 'Dokumenty'], [['archive'], 'Archiwa'], [['code'], 'Kod'], [['app'], 'Programy'],
]
const SIZE_GROUPS: [number, string][] = [
  [0, 'Puste'], [16 << 10, 'Malutkie (do 16 KB)'], [1 << 20, 'Małe (16 KB – 1 MB)'], [128 << 20, 'Średnie (1 – 128 MB)'],
  [1 << 30, 'Duże (128 MB – 1 GB)'], [4 * 2 ** 30, 'Ogromne (1 – 4 GB)'], [Infinity, 'Gigantyczne (ponad 4 GB)'],
]
export const takenOf = (e: Entry) => mediaInfo.get(e.path)?.taken || e.modified

/** [ranga, etykieta] – ranga ustala kolejność grup (rosnąco = „naturalna”). */
function groupOf(e: Entry, g: GroupKey): [number | string, string] {
  switch (g) {
    case 'name': {
      const c = e.name[0]?.toUpperCase() ?? ''
      if (c >= 'A' && c <= 'H') return [1, 'A – H']
      if (c >= 'I' && c <= 'P') return [2, 'I – P']
      if (c >= 'Q' && c <= 'Z') return [3, 'Q – Z']
      if (c >= '0' && c <= '9') return [0, '0 – 9']
      return [4, 'Inne']
    }
    case 'modified': return dateGroup(e.modified)
    case 'created': return dateGroup(e.created)
    case 'taken': return e.dir ? [-1, 'Foldery'] : dateGroup(takenOf(e))
    case 'type': { const l = typeLabel(e); return [e.dir ? '' : l.toLowerCase(), l] }
    case 'kind': {
      const k = kindOf(e)
      const i = KIND_LABEL.findIndex(([ks]) => ks.includes(k))
      return i >= 0 ? [i, KIND_LABEL[i][1]] : [50, 'Inne']
    }
    case 'size': {
      if (e.dir) return [-1, 'Foldery']
      const i = SIZE_GROUPS.findIndex(([max]) => (max === 0 ? e.size === 0 : e.size < max))
      return [i, SIZE_GROUPS[i][1]]
    }
    default: return [0, '']
  }
}
/** Domyślnie: daty od najnowszych, rozmiary od największych, reszta alfabetycznie. */
const GROUP_DESC: Partial<Record<GroupKey, boolean>> = { size: true }

/** Odłączony czytnik / napęd bez nośnika – można go ukryć w ustawieniach. */
export const driveShown = (d: Drive) => !settings.hideOfflineDrives || d.online || (d.kind !== 'removable' && d.kind !== 'cdrom')
export type NetKind = 'smb' | 'webdav' | 'ftp' | 'sftp'
/** Zapamiętane serwery FTP (bez haseł – te są w Menedżerze poświadczeń Windows). */
export interface NetPlace { id: string; name: string; path: string }
export interface SavedSearch { id: string; name: string; root: string; query: string }
export type KindFilter = 'all' | 'media' | 'image' | 'video' | 'doc' | 'audio' | 'archive'
export interface FolderPrefs { view?: View; sort?: SortKey; desc?: boolean; group?: GroupKey }

let seq = 0
const uid = () => `t${Date.now().toString(36)}${(seq++).toString(36)}`

export const HOME = 'ofp:home'
export const SYNC = 'ofp:sync'
export const SETTINGS = 'ofp:settings'
/** Zajętość i porządki: „ofp:space|C:\folder”. */
export const SPACE = 'ofp:space'
export const openSpace = (t: Tab, path: string) => navigate(t, `${SPACE}|${path}`)

export function specialTitle(p: string) {
  if (p === HOME) return 'Komputer'
  if (p === SYNC) return 'Synchronizacja'
  if (p === SETTINGS) return 'Ustawienia'
  if (p.startsWith(SPACE + '|')) return `Zajętość: ${basename(p.slice(SPACE.length + 1)) || p.slice(SPACE.length + 1)}`
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
  /** Wybrane filtry dopasowane do zawartości (format, kadr, data…). */
  facets = new SvelteSet<string>()
  view = $state<View>('list')
  sort = $state<SortKey>('name')
  desc = $state(false)
  group = $state<GroupKey>('none')
  /** Zwinięte grupy (etykiety). */
  collapsed = new SvelteSet<string>()
  /** Zaznaczenie zapamiętane dla każdego folderu – wraca po powrocie do niego. */
  selMemory = new Map<string, { paths: string[]; focus: string | null }>()
  renaming = $state<string | null>(null)
  search = $state<{ query: string; id: number; running: boolean; scanned: number; results: Entry[]; chips: string[]; test: ((e: Entry) => boolean) | null } | null>(null)
  scrollTop = 0
  scrollMemory = new Map<string, number>()
  /** Zmienia się po każdym przeładowaniu, żeby widok mógł odświeżyć miniatury. */
  stamp = $state(0)
  /** Przypięta karta: zawsze z lewej, wąska, nie zamyka się przypadkiem i wraca po każdym uruchomieniu. */
  pinned = $state(false)

  source = $derived(this.search ? this.search.results : this.entries)

  /** Kategorie obecne w folderze (do pokazania tylko sensownych przycisków filtra). */
  kindCounts = $derived.by(() => {
    const c: Partial<Record<KindFilter, number>> = {}
    for (const e of this.source) {
      if (e.dir || (!settings.showHidden && (e.hidden || e.system))) continue
      const k = kindOf(e)
      for (const [f, ks] of Object.entries(KIND_GROUPS) as [Exclude<KindFilter, 'all'>, Kind[]][]) if (ks.includes(k)) c[f] = (c[f] ?? 0) + 1
    }
    return c
  })

  /** Filtry dopasowane do zawartości bieżącej kategorii. */
  facetList: Facet[] = $derived.by(() => {
    media.ver
    const kf = this.kindFilter
    const group = kf === 'all' ? null : KIND_GROUPS[kf]
    const base = this.source.filter((e) => !e.dir && (settings.showHidden || !(e.hidden || e.system)) && (!group || group.includes(kindOf(e))))
    return facetsFor(base, (e) => mediaInfo.get(e.path), [...this.facets])
  })

  grouped: { items: Entry[]; groups: Group[] | null } = $derived.by(() => {
    const hidden = settings.showHidden
    const pq = parseQuery(this.filter)
    const f = pq.text.trim().toLowerCase()
    const kf = this.kindFilter
    const group = kf === 'all' ? null : KIND_GROUPS[kf]
    if (this.facets.size) media.ver
    const ft = facetTest(this.facets, (e) => mediaInfo.get(e.path))
    const arr = this.source.filter(
      (e) =>
        (hidden || !(e.hidden || e.system)) &&
        (!f || e.name.toLowerCase().includes(f)) &&
        (!pq.test || pq.test(e)) &&
        (!group || (!e.dir && group.includes(kindOf(e)))) &&
        (!ft || ft(e)),
    )
    const dir = this.desc ? -1 : 1
    const key = this.sort
    // Klucze z metadanych przeliczamy, gdy dojdą kolejne partie.
    const g = this.group
    if (key === 'taken' || key === 'duration' || key === 'dims' || g === 'taken') media.ver
    if (key === 'size' && settings.folderSizes) dirSizeVer.ver
    const mi = (e: Entry) => mediaInfo.get(e.path)
    const cmp = (a: Entry, b: Entry) => {
      let r = 0
      if (key === 'modified') r = a.modified - b.modified
      else if (key === 'created') r = a.created - b.created
      else if (key === 'size') r = sizeOf(a) - sizeOf(b)
      else if (key === 'type') r = collator.compare(a.ext, b.ext)
      else if (key === 'taken') r = takenOf(a) - takenOf(b)
      else if (key === 'duration') r = (mi(a)?.dur ?? 0) - (mi(b)?.dur ?? 0)
      else if (key === 'dims') r = (mi(a)?.w ?? 0) * (mi(a)?.h ?? 0) - (mi(b)?.w ?? 0) * (mi(b)?.h ?? 0)
      if (r === 0) r = collator.compare(a.name, b.name)
      return r * dir
    }
    const base = (a: Entry, b: Entry) => (settings.foldersFirst && a.dir !== b.dir ? (a.dir ? -1 : 1) : cmp(a, b))
    if (g === 'none') {
      arr.sort(base)
      return { items: arr, groups: null }
    }
    // Grupy: kolejność grup, w środku zwykłe sortowanie. Sortowanie po tym samym kluczu odwraca grupy.
    const keyed = arr.map((e) => ({ e, gk: groupOf(e, g) }))
    const gdesc = (GROUP_DESC[g] ?? false) !== (key === g || (g === 'name' && key === 'name') ? this.desc : false)
    const gd = gdesc ? -1 : 1
    keyed.sort((x, y) => {
      const a = x.gk[0], b = y.gk[0]
      const r = typeof a === 'number' && typeof b === 'number' ? a - b : collator.compare(String(a), String(b))
      return r !== 0 ? r * gd : base(x.e, y.e)
    })
    const groups: Group[] = []
    keyed.forEach((k, i) => {
      const last = groups[groups.length - 1]
      if (last && last.label === k.gk[1]) last.end = i + 1
      else groups.push({ key: k.gk[1], label: k.gk[1], start: i, end: i + 1 })
    })
    return { items: keyed.map((k) => k.e), groups }
  })

  items = $derived(this.grouped.items)
  /** Grupy bieżącej listy (null – bez grupowania). */
  groups = $derived(this.grouped.groups)

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
  hideOfflineDrives: false,
  showRecent: true,
  folderSizes: true,
  showQuickAccess: true,
  theme: 'system' as 'system' | 'dark' | 'light',
  /** Krycie wyskakujących okienek w procentach (60–100). */
  popupOpacity: 100,
  /** Efekt tła okna z Windows 11. */
  windowEffect: 'none' as 'none' | 'mica' | 'acrylic',
  /** Kolor akcentu: 'mono' (biel/czerń), 'system' (z Windows) albo kolor #rrggbb. */
  accent: 'mono' as string,
  shelfOpen: false,
  terminalOpen: false,
  terminalHeight: 260,
  startup: 'restore' as 'restore' | 'home' | 'folder',
  startupFolder: '',
  /** Kolejność sekcji ekranu Komputer (foldery z zawartością mają id „dir:<ścieżka>”). */
  homeOrder: [] as string[],
  /** Foldery pokazywane na ekranie Komputer razem z zawartością. */
  homeDirs: [] as string[],
  recentPinned: [] as string[],
  savedSearches: [] as SavedSearch[],
  netPlaces: [] as NetPlace[],
  listCols: ['modified', 'type', 'size'] as ListCol[],
  listColW: {} as Partial<Record<ListCol, number>>,
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
  /** Tryb, w którym otwiera się paleta: wszystko albo od razu polecenia (Ctrl+Shift+P). */
  paletteMode: 'all' as 'all' | 'cmd',
  sidebarOpen: true,
  ready: false,
  update: null as { latest: string; url: string } | null,
  /** Pobieranie aktualizacji: postęp 0–1, null gdy nic się nie dzieje. */
  updating: null as number | null,
  dragging: null as { paths: string[]; fromShelf?: boolean } | null,
  /** Schowek plików (półka) – osobny od schowka Windows. */
  shelf: [] as Entry[],
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
  | { type: 'bulkRename'; entries: Entry[] }
  | { type: 'network'; kind?: NetKind; host?: string; error?: string }
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

export function closeTab(id: string, force = false) {
  const i = app.tabs.findIndex((t) => t.id === id)
  if (i < 0) return
  const t = app.tabs[i]
  if (t.pinned && !force) return
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

export function togglePin(t: Tab) {
  t.pinned = !t.pinned
  // Przypięte karty trzymamy razem na początku paska.
  const i = app.tabs.indexOf(t)
  app.tabs.splice(i, 1)
  app.tabs.splice(app.tabs.filter((x) => x.pinned).length, 0, t)
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
  t.group = p?.group ?? 'none'
  t.collapsed.clear()
}

export function savePrefs(t: Tab) {
  if (t.special) return
  app.folderPrefs[t.path.toLowerCase()] = { view: t.view, sort: t.sort, desc: t.desc, group: t.group }
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
    t.error = String(e).replace(/^AUTH: /, '')
    // Serwer FTP bez zapamiętanego hasła – od razu pytamy o dane logowania.
    if (isRemote(path) && String(e).startsWith('AUTH') && !app.dialog) {
      app.dialog = { type: 'network', kind: 'ftp', host: path.slice(6).split('\\')[0], error: t.error }
    }
  } finally {
    if (t.path === path) t.loading = false
  }
}

export function navigate(t: Tab, path: string, push = true) {
  if (!path) return
  if (/^[a-zA-Z]:$/.test(path)) path += '\\'
  if (!t.special) {
    t.scrollMemory.set(t.path.toLowerCase(), t.scrollTop)
    const f = t.items[t.focus]
    t.selMemory.set(t.path.toLowerCase(), { paths: [...t.selected], focus: f && t.selected.has(f.path) ? f.path : null })
  }
  const prev = t.path
  t.path = path
  t.search = null
  t.filter = ''
  t.kindFilter = 'all'
  t.facets.clear()
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
    if (t.path !== path) return
    const mem = t.selMemory.get(path.toLowerCase())
    if (mem && !cameFrom) {
      // Powrót do folderu przywraca poprzednie zaznaczenie.
      const alive = new Set(t.items.map((e) => e.path))
      for (const p of mem.paths) if (alive.has(p)) t.selected.add(p)
      const i = mem.focus ? t.items.findIndex((e) => e.path === mem.focus) : -1
      t.focus = i
      t.anchor = i
    } else if (cameFrom) {
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
  if (e.dir || isBrowsableArchive(e)) {
    if (newTabToo) newTab(e.path, false, t.id)
    else navigate(t, e.path)
    return
  }
  // Plik z serwera FTP: pobieramy do folderu tymczasowego i otwieramy.
  if (isRemote(e.path)) {
    toast(`Pobieram „${e.name}”…`)
    api.remoteOpen(e.path).catch((err) => toast(String(err), 'error'))
    return
  }
  // Plik z archiwum: wypakowujemy do folderu tymczasowego i otwieramy w domyślnym programie.
  if (inArchive(e.path)) {
    api.openArchived(e.path).catch((err) => toast(String(err), 'error'))
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

/** Skrót (.lnk): przechodzi do folderu, w którym leży cel, i zaznacza go – jak „Otwórz lokalizację pliku” w Windows. */
export async function openShortcutTarget(t: Tab, e: Entry, newTabToo = false) {
  const props = await api.shellProps(e.path).catch(() => [])
  const target = props.find((p) => p.key === 'System.Link.TargetParsingPath')?.value
  if (!target) return toast('Nie udało się odczytać celu skrótu', 'error')
  const st = await api.stat(target).catch(() => null)
  if (!st) return toast(`Cel skrótu nie istnieje: ${target}`, 'error')
  const dir = parent(st.path)
  if (!dir) return navigate(t, st.path)
  const tab = newTabToo ? newTab(dir, true, t.id) : t
  if (!newTabToo) navigate(tab, dir)
  // Po wczytaniu folderu zaznaczamy cel.
  const wait = setInterval(() => {
    if (!tab.loading && samePath(tab.path, dir)) {
      clearInterval(wait)
      selectOnly(tab, st.path)
    }
  }, 60)
  setTimeout(() => clearInterval(wait), 5000)
}

export function openViewer(t: Tab, e: Entry) {
  const list = t.items.filter((x) => !x.dir)
  app.viewer = { list, index: Math.max(0, list.findIndex((x) => x.path === e.path)) }
}

// ---------- Sortowanie i grupowanie ----------

export function setSort(t: Tab, k: SortKey) {
  if (t.sort === k) t.desc = !t.desc
  else {
    t.sort = k
    t.desc = k !== 'name' && k !== 'type'
  }
  if (k === 'taken' || k === 'duration' || k === 'dims') ensureMedia(t.items)
  savePrefs(t)
}
export function setGroup(t: Tab, g: GroupKey) {
  t.group = g
  t.collapsed.clear()
  if (g === 'taken') ensureMedia(t.items)
  savePrefs(t)
}

/** Pozycje „Sortuj według” i „Grupuj według” – wspólne dla paska narzędzi i menu tła. */
export function sortMenuItems(t: Tab): MenuItem[] {
  const opt = (k: SortKey) => ({ label: (t.sort === k ? (t.desc ? '↓ ' : '↑ ') : '   ') + SORT_LABEL[k], run: () => setSort(t, k) })
  const grp = (g: GroupKey) => ({ label: (t.group === g ? '✓ ' : '   ') + GROUP_LABEL[g], run: () => setGroup(t, g) })
  return [
    { label: 'Sortuj według', icon: 'sort', sub: (Object.keys(SORT_LABEL) as SortKey[]).map(opt) },
    { label: 'Grupuj według', icon: 'list', sub: (Object.keys(GROUP_LABEL) as GroupKey[]).map(grp) },
  ]
}

// ---------- Wyszukiwanie ----------

let unlistenSearch: (() => void) | null = null
export async function startSearch(t: Tab, query: string) {
  query = query.trim()
  if (!query || t.special) return
  // Słowa idą do wyszukiwarki, filtry (typ:, rozmiar:, data:…) sprawdzamy przy odbiorze wyników.
  const pq = parseQuery(query)
  const id = await api.search(t.path, pq.text, settings.showHidden)
  t.search = { query, id, running: true, scanned: 0, results: [], chips: pq.chips, test: pq.test }
  t.selected.clear()
  t.focus = -1
}
export async function saveSearch(t: Tab) {
  if (!t.search) return
  const root = t.path
  const name = await prompt('Zapisz wyszukiwanie', `${t.search.query} w ${basename(root) || root}`, 'Zapisz', false, 'Pojawi się w panelu bocznym nad przypiętymi folderami.')
  if (!name) return
  settings.savedSearches = [...settings.savedSearches, { id: uid(), name, root, query: t.search.query }]
  toast(`Zapisano „${name}”`, 'ok')
}
export function runSaved(t: Tab, s: SavedSearch) {
  if (!samePath(t.path, s.root)) navigate(t, s.root)
  // Nawigacja czyści wyszukiwanie – startujemy po niej.
  queueMicrotask(() => startSearch(t, s.query))
}
export function removeSaved(id: string) {
  settings.savedSearches = settings.savedSearches.filter((s) => s.id !== id)
}
export async function renameSaved(s: SavedSearch) {
  const name = await prompt('Zmień nazwę wyszukiwania', s.name, 'Zapisz')
  if (name) settings.savedSearches = settings.savedSearches.map((x) => (x.id === s.id ? { ...x, name } : x))
}

export function closeSearch(t: Tab) {
  api.cancelSearch()
  t.search = null
  t.selected.clear()
}

// ---------- Operacje na plikach ----------

/** Wypakowanie elementów archiwum (przeciąganie, kopiowanie, wklejanie z archiwum). */
export async function extractTo(sources: string[], dest: string) {
  if (dest.startsWith('ofp:') || inArchive(dest + '\\')) return toast('Archiwa są tylko do odczytu', 'error')
  toast(`Wypakowuję ${sources.length === 1 ? `„${basename(sources[0])}”` : sources.length + ' elem.'}…`)
  try {
    const out = await api.extractItems(sources, dest)
    toast(`Wypakowano do „${basename(dest)}”`, 'ok')
    for (const t of app.tabs) if (!t.special && samePath(t.path, dest)) load(t, true)
    return out
  } catch (e) {
    toast(String(e), 'error')
  }
}

export async function extractHere(path: string) {
  try {
    const out = await api.extractHere(path)
    toast(`Wypakowano do „${basename(out)}”`, 'ok')
    for (const t of app.tabs) if (!t.special && samePath(t.path, parent(path) ?? '')) load(t, true)
  } catch (e) {
    toast(String(e), 'error')
  }
}

/** Kopiowanie z serwera FTP / na serwer FTP (bez przenoszenia – oryginał zostaje). */
async function remoteTransfer(sources: string[], dest: string) {
  const what = sources.length === 1 ? `„${basename(sources[0])}”` : `${sources.length} elem.`
  try {
    if (isRemote(dest)) {
      toast(`Wysyłam ${what}…`)
      await api.remoteUpload(sources, dest)
      toast(`Wysłano ${what}`, 'ok')
    } else {
      toast(`Pobieram ${what}…`)
      const dirs = new Set(app.tabs.flatMap((t) => t.items.filter((e) => e.dir).map((e) => e.path)))
      await api.remoteDownload(sources.map((s) => [s, dirs.has(s)] as [string, boolean]), dest)
      toast(`Pobrano ${what}`, 'ok')
    }
    for (const t of app.tabs) if (!t.special && samePath(t.path, dest)) load(t, true)
  } catch (e) {
    toast(String(e).replace(/^AUTH: /, ''), 'error')
  }
}

export async function transfer(sources: string[], dest: string, mode: 'copy' | 'move') {
  if (isRemote(dest) || sources.some(isRemote)) {
    if (isRemote(dest) && sources.some(isRemote)) return toast('Kopiowanie między serwerami FTP nie jest obsługiwane', 'error')
    return remoteTransfer(sources, dest)
  }
  if (sources.some(inArchive)) {
    await extractTo(sources.filter(inArchive), dest)
    sources = sources.filter((s) => !inArchive(s))
    if (!sources.length) return
  }
  if (inArchive(dest + '\\')) return toast('Archiwa są tylko do odczytu – nie można do nich kopiować', 'error')
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
  if (sel.some((e) => inArchive(e.path))) return toast('Archiwa są tylko do odczytu', 'error')
  if (sel.some((e) => isRemote(e.path))) {
    const what = sel.length === 1 ? `„${sel[0].name}”` : `${sel.length} elementów`
    if (!(await confirm('Usunąć z serwera?', `${what} zostanie usunięte z serwera FTP bez możliwości przywrócenia.`, 'Usuń', true))) return
    try {
      await api.remoteDelete(sel.map((e) => [e.path, e.dir] as [string, boolean]))
      toast('Usunięto z serwera', 'ok')
    } catch (e) {
      toast(String(e), 'error')
    }
    t.selected.clear()
    return load(t, true)
  }
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
  if (inArchive(t.path + '\\')) return toast('Archiwa są tylko do odczytu', 'error')
  const name = await prompt('Nowy folder', 'Nowy folder', 'Utwórz')
  if (!name) return
  try {
    const p = isRemote(t.path) ? await api.remoteMkdir(t.path, name) : await api.mkdir(t.path, name)
    await load(t)
    selectOnly(t, p)
  } catch (e) {
    toast(String(e), 'error')
  }
}

/** Nowy folder i przeniesienie do niego zaznaczonych elementów. */
export async function newFolderWith(t: Tab) {
  const sel = t.selectedEntries.map((e) => e.path)
  if (t.special || !sel.length) return
  const name = await prompt('Nowy folder z zaznaczonych', `Nowy folder (${sel.length} elem.)`, 'Utwórz')
  if (!name) return
  try {
    const p = await api.mkdir(t.path, name)
    await api.startOp('move', sel, p, 'rename')
    await load(t)
    selectOnly(t, p)
  } catch (e) {
    toast(String(e), 'error')
  }
}

/** Archiwum ZIP z zaznaczonych elementów, obok nich. */
export async function zipSelection(t: Tab) {
  const sel = t.selectedEntries
  if (t.special || !sel.length) return
  const base = sel.length === 1 ? sel[0].name.replace(/\.[^.]+$/, '') : basename(t.path) || 'Archiwum'
  const name = await prompt('Nowe archiwum ZIP', `${base}.zip`, 'Spakuj', true)
  if (!name) return
  toast(`Pakuję ${sel.length} elem.…`)
  try {
    const p = await api.createZip(sel.map((e) => e.path), join(t.path, name.toLowerCase().endsWith('.zip') ? name : name + '.zip'))
    await load(t)
    selectOnly(t, p)
    toast(`Utworzono „${basename(p)}”`, 'ok')
  } catch (e) {
    toast(String(e), 'error')
  }
}

export async function newFile(t: Tab, def = 'Nowy dokument tekstowy.txt', title = 'Nowy plik') {
  if (t.special) return
  if (inArchive(t.path + '\\')) return toast('Archiwa są tylko do odczytu', 'error')
  const name = await prompt(title, def, 'Utwórz', true)
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
  if (inArchive(e.path)) return toast('Archiwa są tylko do odczytu', 'error')
  try {
    const p = isRemote(e.path) ? await api.remoteRename(e.path, name) : await api.rename(e.path, name)
    await load(t, true)
    selectOnly(t, p)
  } catch (err) {
    toast(String(err), 'error')
  }
}

export async function undo() {
  try {
    toast(await api.undo(), 'ok', { label: 'Ponów', run: redo })
  } catch (e) {
    toast(String(e))
  }
}
export async function redo() {
  try {
    toast(await api.redo(), 'ok', { label: 'Cofnij', run: undo })
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
export function toggleHomeDir(path: string) {
  if (isHomeDir(path)) {
    settings.homeDirs = settings.homeDirs.filter((p) => !samePath(p, path))
  } else {
    settings.homeDirs = [...settings.homeDirs, path]
    toast(`„${basename(path)}” pojawi się na ekranie Komputer`, 'ok')
  }
}
export const isHomeDir = (path: string) => settings.homeDirs.some((p) => samePath(p, path))

export function removeRecent(path: string) {
  app.recent = app.recent.filter((r) => !samePath(r, path))
  settings.recentPinned = settings.recentPinned.filter((r) => !samePath(r, path))
}
export function toggleRecentPin(path: string) {
  settings.recentPinned = settings.recentPinned.some((r) => samePath(r, path))
    ? settings.recentPinned.filter((r) => !samePath(r, path))
    : [path, ...settings.recentPinned]
}
export function clearRecent() {
  app.recent = []
}

export function addNetPlace(path: string, name: string) {
  if (settings.netPlaces.some((p) => samePath(p.path, path))) return
  settings.netPlaces = [...settings.netPlaces, { id: uid(), name, path }]
}
export function removeNetPlace(p: NetPlace, forgetPassword: boolean) {
  settings.netPlaces = settings.netPlaces.filter((x) => x.id !== p.id)
  if (forgetPassword) api.ftpForget(p.path)
}

/** Aktualizacja z aplikacji: wersja zainstalowana pobiera instalator i instaluje się sama, przenośna otwiera stronę wydania. */
export async function updateNow() {
  if (app.updating !== null) return
  const info = await api.integration().catch(() => null)
  if (!info?.installed) {
    if (app.update?.url) api.open(app.update.url)
    else toast('Wersja przenośna – pobierz nową ze strony wydania')
    return
  }
  app.updating = 0
  const off = await on<{ done: number; total: number }>('update-progress', (p) => (app.updating = p.total ? p.done / p.total : 0))
  try {
    const setup = await api.downloadUpdate()
    toast('Instaluję aktualizację – aplikacja uruchomi się ponownie', 'ok')
    await new Promise((r) => setTimeout(r, 800))
    await api.installUpdate(setup)
  } catch (e) {
    toast(`Aktualizacja nie powiodła się: ${e}`, 'error')
  } finally {
    off()
    app.updating = null
  }
}

export function shelfAdd(entries: Entry[]) {
  const have = new Set(app.shelf.map((e) => e.path.toLowerCase()))
  const add = entries.filter((e) => !have.has(e.path.toLowerCase()) && !isRemote(e.path) && !inArchive(e.path))
  if (!add.length) return
  app.shelf = [...app.shelf, ...add]
  settings.shelfOpen = true
  toast(`Dodano do schowka plików: ${add.length === 1 ? add[0].name : add.length + ' elem.'}`, 'ok')
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

/** Dyski, które da się bezpiecznie wysunąć (pendrive, karta, dysk USB, płyta). */
export const ejectable = (d: Drive) => d.online && (d.kind === 'removable' || d.kind === 'cdrom')
export const lowSpace = (d: Drive) => d.online && d.total > 0 && d.free / d.total < 0.1

export async function ejectDrive(d: Drive) {
  // Karty otwarte na tym dysku przechodzą do „Komputer” – inaczej Windows nie pozwoli wysunąć.
  for (const t of app.tabs) if (!t.special && volumeOf(t.path) === volumeOf(d.path)) navigate(t, HOME)
  await new Promise((r) => setTimeout(r, 150))
  try {
    await api.eject(d.path)
    toast(`${d.label || d.letter} – można bezpiecznie odłączyć`, 'ok')
  } catch (e) {
    toast(`Nie udało się wysunąć ${d.letter}: ${e}`, 'error')
  }
  refreshDrivesSoon()
}

async function unmapDrive(d: Drive, forget: boolean) {
  if (forget && !(await confirm('Zapomnieć dysk?', `${d.letter} (${d.remote}) nie będzie przywracany przy logowaniu.`, 'Rozłącz', true))) return
  try {
    await api.unmapDrive(d.letter, forget)
    toast(`Rozłączono ${d.letter}`, 'ok')
    refreshDrives()
  } catch (err) {
    toast(String(err), 'error')
  }
}

function nativeMenu(paths: string[], ev: MouseEvent) {
  api.nativeMenu(paths, ev.clientX, ev.clientY, ev.shiftKey).then((done) => done && setTimeout(refreshDrives, 600)).catch((err) => toast(String(err), 'error'))
}

/** Menu kontekstowe dysku – wspólne dla panelu bocznego i ekranu „Komputer”. */
export function showDriveMenu(ev: MouseEvent, d: Drive, open: (path: string) => void, openNew: (path: string) => void) {
  ev.preventDefault()
  if (ev.shiftKey) return nativeMenu([d.path], ev)
  const items: MenuItem[] = [
    { label: 'Otwórz', icon: 'drive', kbd: 'Enter', disabled: !d.online, run: () => open(d.path) },
    { label: 'Otwórz w nowej karcie', icon: 'plus', kbd: 'Ctrl+klik', disabled: !d.online, run: () => openNew(d.path) },
    { sep: true },
    { label: 'Kopiuj ścieżkę', icon: 'link', run: () => { navigator.clipboard.writeText(d.path); toast('Skopiowano ścieżkę') } },
    { label: 'Terminal tutaj', icon: 'terminal', disabled: !d.online, run: () => api.terminal(d.path) },
    { label: 'Zajętość i porządki', icon: 'grid', disabled: !d.online, run: () => openSpace(activeTab(), d.path) },
    { label: 'Otwórz w Eksploratorze', icon: 'open', disabled: !d.online, run: () => api.open(d.path) },
  ]
  if (d.kind !== 'network' && d.online) items.push({ label: 'Oczyszczanie dysku…', icon: 'trash', run: () => api.diskCleanup(d.letter).catch((e) => toast(String(e), 'error')) })
  if (ejectable(d)) items.push({ sep: true }, { label: 'Wysuń bezpiecznie', icon: 'eject', run: () => ejectDrive(d) })
  if (d.kind === 'network') {
    items.push(
      { sep: true },
      { label: 'Rozłącz', icon: 'unlink', run: () => unmapDrive(d, false) },
      { label: 'Rozłącz i zapomnij', icon: 'unlink', danger: true, run: () => unmapDrive(d, true) },
    )
  }
  items.push(
    { sep: true },
    { label: 'Właściwości systemowe', icon: 'info', run: () => api.systemProps(d.path) },
    { label: 'Pokaż więcej opcji (menu Windows)', icon: 'more', kbd: 'Shift+PPM', run: () => nativeMenu([d.path], ev) },
  )
  app.menu = { x: ev.clientX, y: ev.clientY, items }
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
    tabs: app.tabs.filter((t) => !t.search).map((t) => ({ path: t.path, pinned: t.pinned })),
    active: Math.max(0, app.tabs.findIndex((t) => t.id === app.panes[app.focusedPane])),
    split: app.split,
    sidebarOpen: app.sidebarOpen,
    shelf: app.shelf,
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
    app.shelf = s.shelf ?? []
    const saved: { path: string; pinned: boolean }[] = (s.tabs ?? []).map((x: string | { path: string; pinned?: boolean }) =>
      typeof x === 'string' ? { path: x, pinned: false } : { path: x.path, pinned: !!x.pinned },
    )
    // Przypięte karty wracają zawsze; resztę ustala ustawienie „Po uruchomieniu”.
    const pinned = saved.filter((x) => x.pinned)
    const rest =
      settings.startup === 'restore' ? saved.filter((x) => !x.pinned)
      : settings.startup === 'folder' && settings.startupFolder ? [{ path: settings.startupFolder, pinned: false }]
      : [{ path: HOME, pinned: false }]
    const list = [...pinned, ...rest]
    if (!list.length) list.push({ path: HOME, pinned: false })
    for (const x of list) newTab(x.path, false).pinned = x.pinned
    const restoring = settings.startup === 'restore'
    const act = restoring ? app.tabs[Math.min(s.active ?? 0, app.tabs.length - 1)] : app.tabs[pinned.length] ?? app.tabs[0]
    app.panes = [act.id]
    if (restoring && s.split && app.tabs.length > 1) {
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
    // Zmiana w folderze unieważnia policzone rozmiary jego i wszystkich nadrzędnych.
    for (const d of dirs) for (const k of [...dirSizes.keys()]) if (d.toLowerCase().startsWith(k)) dirSizes.delete(k)
    for (const t of app.tabs) {
      if (!t.special && !t.search && dirs.some((d) => samePath(d, t.path))) load(t, true)
    }
  })
  on('drives-changed', () => refreshDrives())
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
        const test = t.search.test
        t.search.results = [...t.search.results, ...(test ? b.items.filter(test) : b.items)]
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
      const dirs = [...new Set(app.tabs.filter((t) => !t.special && !isRemote(t.path) && !inArchive(t.path + '\\')).map((t) => t.path))]
      api.watch(dirs)
    })
    $effect(() => {
      JSON.stringify(settings)
      app.favorites.length
      app.recent[0]
      app.recent.length
      app.tabs.map((t) => t.path + (t.pinned ? '*' : '')).join('|')
      app.panes.join('|')
      app.split
      app.sidebarOpen
      app.shelf.length
      Object.keys(app.folderPrefs).length
      persist()
    })
    $effect(() => {
      api.closeToTray(settings.closeToTray)
    })
    $effect(() => {
      const fx = settings.windowEffect
      document.documentElement.dataset.effect = fx === 'none' ? '' : fx
      const w = getCurrentWindow()
      ;(fx === 'none' ? w.clearEffects() : w.setEffects({ effects: [fx === 'mica' ? Effect.Mica : Effect.Acrylic] })).catch(() => {})
    })
    $effect(() => {
      const want = settings.accent
      const root = document.documentElement.style
      const apply = (hex: string | null) => {
        if (!hex || !/^#[0-9a-f]{6}$/i.test(hex)) {
          for (const v of ['--accent', '--on-accent', '--accent-hover']) root.removeProperty(v)
          return
        }
        const n = parseInt(hex.slice(1), 16)
        const lum = (0.2126 * ((n >> 16) & 255) + 0.7152 * ((n >> 8) & 255) + 0.0722 * (n & 255)) / 255
        root.setProperty('--accent', hex)
        root.setProperty('--on-accent', lum > 0.6 ? '#000' : '#fff')
        root.setProperty('--accent-hover', `color-mix(in srgb, ${hex} 85%, ${lum > 0.6 ? '#000' : '#fff'})`)
      }
      if (want === 'system') api.systemAccent().then(apply).catch(() => apply(null))
      else apply(want === 'mono' ? null : want)
    })
    $effect(() => {
      const a = Math.max(50, Math.min(100, settings.popupOpacity)) / 100
      const root = document.documentElement.style
      root.setProperty('--popup-alpha', String(a))
      // Przy przezroczystości rozmywamy to, co jest pod okienkiem – inaczej tekst się zlewa.
      root.setProperty('--popup-blur', a < 1 ? `blur(${Math.round(8 + (1 - a) * 40)}px) saturate(1.3)` : 'none')
    })
    $effect(() => {
      const mq = matchMedia('(prefers-color-scheme: light)')
      const apply = () => (document.documentElement.dataset.theme = settings.theme === 'system' ? (mq.matches ? 'light' : 'dark') : settings.theme)
      apply()
      mq.addEventListener('change', apply)
      return () => mq.removeEventListener('change', apply)
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
