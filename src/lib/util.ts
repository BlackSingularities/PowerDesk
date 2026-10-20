import type { Entry } from './api'

export type Kind = 'folder' | 'image' | 'raw' | 'video' | 'audio' | 'pdf' | 'doc' | 'sheet' | 'slides' | 'text' | 'code' | 'archive' | 'app' | 'other'

const KIND: Record<string, Kind> = {}
const reg = (k: Kind, list: string) => list.split(' ').forEach((e) => (KIND[e] = k))
reg('image', 'jpg jpeg jfif png gif webp bmp ico avif svg tif tiff heic heif jxl')
reg('raw', 'cr2 cr3 nef nrw arw srf sr2 dng raf orf rw2 pef srw x3f 3fr erf kdc mrw mos iiq rwl')
reg('video', 'mp4 m4v mov mkv webm avi wmv mts m2ts ts 3gp flv mpg mpeg vob ogv')
reg('audio', 'mp3 flac wav m4a aac ogg opus wma aiff ape alac')
reg('pdf', 'pdf')
reg('doc', 'doc docx docm odt rtf dotx pages epub')
reg('sheet', 'xls xlsx xlsm ods csv numbers')
reg('slides', 'ppt pptx ppsx odp key')
reg('text', 'txt md log ini cfg conf nfo srt vtt')
reg('code', 'js ts jsx tsx json xml html htm css scss py rs kt kts java c h cpp hpp cs go rb php sh ps1 bat cmd yml yaml toml sql lua swift dart vue svelte gradle properties reg')
reg('archive', 'zip 7z rar tar gz tgz bz2 xz zst cab iso')
reg('app', 'exe msi lnk appx msix url')

export function kindOf(e: Pick<Entry, 'dir' | 'ext'>): Kind {
  return e.dir ? 'folder' : KIND[e.ext] ?? 'other'
}

/** Formaty, które WebView2 wyświetla natywnie – resztę (HEIC, RAW, TIFF…) dekoduje Windows (WIC). */
export const WEB_IMAGE = new Set(['jpg', 'jpeg', 'jfif', 'png', 'gif', 'webp', 'bmp', 'ico', 'avif', 'svg'])
export const WEB_VIDEO = new Set(['mp4', 'm4v', 'webm', 'mov', 'ogv', 'mkv'])
export const WEB_AUDIO = new Set(['mp3', 'flac', 'wav', 'm4a', 'aac', 'ogg', 'opus'])
export const OFFICE_TEXT = new Set(['docx', 'docm', 'dotx', 'odt', 'xlsx', 'xlsm', 'ods', 'pptx', 'ppsx', 'odp', 'epub'])

export const isMedia = (e: Pick<Entry, 'dir' | 'ext'>) => {
  const k = kindOf(e)
  return k === 'image' || k === 'raw' || k === 'video'
}

export const KIND_LABEL: Record<Kind, string> = {
  folder: 'Folder', image: 'Obraz', raw: 'Zdjęcie RAW', video: 'Wideo', audio: 'Audio', pdf: 'PDF',
  doc: 'Dokument', sheet: 'Arkusz', slides: 'Prezentacja', text: 'Tekst', code: 'Kod', archive: 'Archiwum',
  app: 'Program', other: 'Plik',
}

export function typeLabel(e: Entry) {
  if (e.dir) return 'Folder'
  const k = kindOf(e)
  return e.ext ? `${KIND_LABEL[k] === 'Plik' ? 'Plik' : KIND_LABEL[k]} ${e.ext.toUpperCase()}` : 'Plik'
}

export function size(n: number, digits = 1) {
  if (!isFinite(n) || n < 0) return '—'
  if (n < 1024) return `${n} B`
  const u = ['KB', 'MB', 'GB', 'TB', 'PB']
  let i = -1
  do {
    n /= 1024
    i++
  } while (n >= 1024 && i < u.length - 1)
  return `${n.toLocaleString('pl-PL', { maximumFractionDigits: n < 10 ? digits : 0 })} ${u[i]}`
}

const dtf = new Intl.DateTimeFormat('pl-PL', { dateStyle: 'short', timeStyle: 'short' })
const dfull = new Intl.DateTimeFormat('pl-PL', { dateStyle: 'full', timeStyle: 'medium' })
export const date = (ms: number) => (ms ? dtf.format(ms) : '—')
export const dateFull = (ms: number) => (ms ? dfull.format(ms) : '—')

const rtf = new Intl.RelativeTimeFormat('pl-PL', { numeric: 'auto' })
export function ago(ms: number) {
  if (!ms) return 'nigdy'
  const s = (ms - Date.now()) / 1000
  const a = Math.abs(s)
  if (a < 45) return 'przed chwilą'
  if (a < 3600) return rtf.format(Math.round(s / 60), 'minute')
  if (a < 86400) return rtf.format(Math.round(s / 3600), 'hour')
  if (a < 86400 * 30) return rtf.format(Math.round(s / 86400), 'day')
  if (a < 86400 * 365) return rtf.format(Math.round(s / (86400 * 30)), 'month')
  return rtf.format(Math.round(s / (86400 * 365)), 'year')
}

export const plural = (n: number, one: string, few: string, many: string) => {
  const n10 = n % 10, n100 = n % 100
  if (n === 1) return one
  if (n10 >= 2 && n10 <= 4 && (n100 < 12 || n100 > 14)) return few
  return many
}
export const items = (n: number) => `${n.toLocaleString('pl-PL')} ${plural(n, 'element', 'elementy', 'elementów')}`

export function basename(p: string) {
  const t = p.replace(/[\\/]+$/, '')
  if (/^[a-zA-Z]:$/.test(t)) return t + '\\'
  const i = t.lastIndexOf('\\')
  return i >= 0 ? t.slice(i + 1) : t
}

/** Serwer FTP przeglądany w aplikacji: ftp:\\serwer\katalog. */
export const isRemote = (p: string) => /^ftp:\\\\/i.test(p)

export function parent(p: string): string | null {
  if (p.startsWith('ofp:')) return null
  const t = p.replace(/[\\/]+$/, '')
  if (/^ftp:\\\\[^\\]+$/i.test(t)) return null
  if (/^[a-zA-Z]:$/.test(t)) return null
  if (/^\\\\[^\\]+\\[^\\]+$/.test(t)) return null
  const i = t.lastIndexOf('\\')
  if (i < 0) return null
  const up = t.slice(0, i)
  return /^[a-zA-Z]:$/.test(up) ? up + '\\' : up
}

export function join(dir: string, name: string) {
  return dir.endsWith('\\') ? dir + name : `${dir}\\${name}`
}

export function crumbs(p: string): { name: string; path: string }[] {
  if (p.startsWith('ofp:lib|')) return [{ name: 'Biblioteki', path: p }, { name: basename(p.slice(8)).replace(/\.library-ms$/i, ''), path: p }]
  if (p.startsWith('ofp:')) return []
  const out: { name: string; path: string }[] = []
  let cur: string | null = p
  while (cur) {
    out.unshift({ name: basename(cur), path: cur })
    cur = parent(cur)
  }
  if (p.startsWith('\\\\') && out.length) out[0].name = out[0].path
  if (isRemote(p) && out.length) out[0].name = 'ftp://' + out[0].path.slice(6)
  return out
}

/** Archiwa, które da się otworzyć jak folder. */
export const BROWSABLE_ARCHIVE = new Set(['zip', '7z'])
export const isBrowsableArchive = (e: Pick<Entry, 'dir' | 'ext'>) => !e.dir && BROWSABLE_ARCHIVE.has(e.ext)
/** Ścieżka prowadzi do wnętrza archiwum (C:\a\x.zip\folder). */
export const inArchive = (p: string) => /\.(zip|7z)\\/i.test(p)

export function samePath(a: string, b: string) {
  return a.replace(/[\\/]+$/, '').toLowerCase() === b.replace(/[\\/]+$/, '').toLowerCase()
}

export function volumeOf(p: string) {
  if (isRemote(p)) return p.split('\\').slice(0, 3).join('\\').toLowerCase()
  if (p.startsWith('\\\\')) return p.split('\\').slice(0, 4).join('\\').toLowerCase()
  return p.slice(0, 2).toLowerCase()
}

/** Naturalne sortowanie jak w Eksploratorze: „plik 2" przed „plik 10". */
export const collator = new Intl.Collator('pl', { numeric: true, sensitivity: 'base' })

export function debounce<A extends unknown[]>(fn: (...a: A) => void, ms: number) {
  let t: ReturnType<typeof setTimeout> | undefined
  return (...a: A) => {
    clearTimeout(t)
    t = setTimeout(() => fn(...a), ms)
  }
}

export function fuzzy(q: string, s: string): number {
  q = q.toLowerCase()
  s = s.toLowerCase()
  if (!q) return 1
  const idx = s.indexOf(q)
  if (idx >= 0) return 1000 - idx - s.length * 0.01
  let qi = 0, score = 0, streak = 0
  for (let i = 0; i < s.length && qi < q.length; i++) {
    if (s[i] === q[qi]) {
      qi++
      streak++
      score += 1 + streak
    } else streak = 0
  }
  return qi === q.length ? score : -1
}

/** Czas trwania „1:05”, „1:02:03”. */
export function duration(ms: number) {
  const t = Math.round(ms / 1000)
  const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), sec = t % 60
  return (h ? `${h}:${String(m).padStart(2, '0')}` : `${m}`) + `:${String(sec).padStart(2, '0')}`
}
