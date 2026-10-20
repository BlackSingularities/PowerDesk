import type { Entry } from './api'
import { kindOf, type Kind } from './util'

/**
 * Zapytanie z filtrami: zwykłe słowa szukają w nazwie, a słowa z dwukropkiem zawężają wynik.
 *   typ:zdjęcia   rozmiar:>100mb   data:tydzień   ext:jpg,png   data:2024   rozmiar:1mb-1gb
 */
export interface Parsed {
  /** Część tekstowa – trafia do wyszukiwarki w Rust (nazwa, * i ?). */
  text: string
  /** Filtr dodatkowy albo null, gdy zapytanie ma same słowa. */
  test: ((e: Entry) => boolean) | null
  /** Rozpoznane filtry – do pokazania w interfejsie. */
  chips: string[]
}

const TYPES: Record<string, Kind[] | 'dir' | 'file'> = {
  zdjecia: ['image', 'raw'], zdjęcia: ['image', 'raw'], zdjecie: ['image', 'raw'], zdjęcie: ['image', 'raw'], foto: ['image', 'raw'], obrazy: ['image', 'raw'], image: ['image', 'raw'],
  raw: ['raw'], wideo: ['video'], video: ['video'], filmy: ['video'], film: ['video'],
  audio: ['audio'], muzyka: ['audio'], music: ['audio'],
  dokumenty: ['pdf', 'doc', 'sheet', 'slides', 'text'], dokument: ['pdf', 'doc', 'sheet', 'slides', 'text'], doc: ['pdf', 'doc', 'sheet', 'slides', 'text'],
  pdf: ['pdf'], arkusze: ['sheet'], prezentacje: ['slides'], tekst: ['text'], kod: ['code'], code: ['code'],
  archiwa: ['archive'], archiwum: ['archive'], zip: ['archive'], programy: ['app'], app: ['app'],
  folder: 'dir', foldery: 'dir', katalog: 'dir', plik: 'file', pliki: 'file', file: 'file',
}
export const TYPE_LABEL: Record<string, string> = { dir: 'foldery', file: 'pliki' }

const UNITS: Record<string, number> = { b: 1, k: 1024, kb: 1024, m: 2 ** 20, mb: 2 ** 20, g: 2 ** 30, gb: 2 ** 30, t: 2 ** 40, tb: 2 ** 40 }
function bytes(s: string): number | null {
  const m = s.trim().replace(',', '.').match(/^(\d+(?:\.\d+)?)\s*([kmgt]?b?)?$/i)
  if (!m) return null
  return parseFloat(m[1]) * (UNITS[(m[2] || 'b').toLowerCase()] ?? 1)
}

const DAY = 86_400_000
function dayStart(t = Date.now()) {
  const d = new Date(t)
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
}
/** Zakres dat [od, do) dla słów: dziś, wczoraj, tydzień, miesiąc, rok, 2024, 2024-05, 2024-05-17, >2024-01-01. */
function dateRange(v: string): [number, number] | null {
  const now = Date.now(), today = dayStart()
  const w = v.toLowerCase()
  if (w === 'dziś' || w === 'dzis' || w === 'today') return [today, Infinity]
  if (w === 'wczoraj' || w === 'yesterday') return [today - DAY, today]
  if (w === 'tydzień' || w === 'tydzien' || w === 'week') return [today - 6 * DAY, Infinity]
  if (w === 'miesiąc' || w === 'miesiac' || w === 'month') return [today - 29 * DAY, Infinity]
  if (w === 'rok' || w === 'year') return [new Date(new Date(now).getFullYear(), 0, 1).getTime(), Infinity]
  const cmp = w.match(/^([<>])(.+)$/)
  if (cmp) {
    const r = dateRange(cmp[2])
    if (!r) return null
    return cmp[1] === '>' ? [r[1] === Infinity ? r[0] : r[1], Infinity] : [0, r[0]]
  }
  const m = w.match(/^(\d{4})(?:-(\d{1,2}))?(?:-(\d{1,2}))?$/)
  if (!m) return null
  const y = +m[1], mo = m[2] ? +m[2] - 1 : null, d = m[3] ? +m[3] : null
  if (mo === null) return [new Date(y, 0, 1).getTime(), new Date(y + 1, 0, 1).getTime()]
  if (d === null) return [new Date(y, mo, 1).getTime(), new Date(y, mo + 1, 1).getTime()]
  return [new Date(y, mo, d).getTime(), new Date(y, mo, d + 1).getTime()]
}

export function parseQuery(q: string): Parsed {
  const words: string[] = []
  const tests: ((e: Entry) => boolean)[] = []
  const chips: string[] = []
  for (const tok of q.trim().split(/\s+/).filter(Boolean)) {
    const m = tok.match(/^([a-ząćęłńóśźż]+):(.+)$/i)
    if (!m) {
      words.push(tok)
      continue
    }
    const key = m[1].toLowerCase(), val = m[2]
    if (key === 'typ' || key === 'type' || key === 'rodzaj') {
      const kinds = val.split(',').map((v) => TYPES[v.toLowerCase()]).filter(Boolean)
      if (!kinds.length) { words.push(tok); continue }
      tests.push((e) => kinds.some((k) => (k === 'dir' ? e.dir : k === 'file' ? !e.dir : !e.dir && k.includes(kindOf(e)))))
      chips.push(`typ: ${val}`)
    } else if (key === 'ext' || key === 'rozszerzenie') {
      const exts = val.toLowerCase().split(',').map((x) => x.replace(/^\./, ''))
      tests.push((e) => !e.dir && exts.includes(e.ext))
      chips.push(`.${exts.join(', .')}`)
    } else if (key === 'rozmiar' || key === 'size') {
      const range = val.match(/^(.+?)-(.+)$/)
      const cmp = val.match(/^([<>]=?)(.+)$/)
      if (range && bytes(range[1]) !== null && bytes(range[2]) !== null) {
        const lo = bytes(range[1])!, hi = bytes(range[2])!
        tests.push((e) => !e.dir && e.size >= lo && e.size <= hi)
      } else if (cmp && bytes(cmp[2]) !== null) {
        const b = bytes(cmp[2])!, op = cmp[1]
        tests.push((e) => !e.dir && (op === '>' ? e.size > b : op === '>=' ? e.size >= b : op === '<' ? e.size < b : e.size <= b))
      } else if (bytes(val) !== null) {
        const b = bytes(val)!
        tests.push((e) => !e.dir && e.size >= b)
      } else { words.push(tok); continue }
      chips.push(`rozmiar ${val}`)
    } else if (key === 'data' || key === 'date' || key === 'zmieniony' || key === 'modified') {
      const r = dateRange(val)
      if (!r) { words.push(tok); continue }
      tests.push((e) => e.modified >= r[0] && e.modified < r[1])
      chips.push(`data: ${val}`)
    } else if (key === 'utworzony' || key === 'created') {
      const r = dateRange(val)
      if (!r) { words.push(tok); continue }
      tests.push((e) => e.created >= r[0] && e.created < r[1])
      chips.push(`utworzony: ${val}`)
    } else words.push(tok)
  }
  return { text: words.join(' '), test: tests.length ? (e) => tests.every((t) => t(e)) : null, chips }
}

/** Podpowiedzi pokazywane pod polem wyszukiwania. */
export const QUERY_HINTS: [string, string][] = [
  ['typ:zdjęcia', 'Tylko zdjęcia'],
  ['typ:wideo', 'Tylko filmy'],
  ['typ:dokumenty', 'Dokumenty'],
  ['typ:folder', 'Tylko foldery'],
  ['rozmiar:>100mb', 'Większe niż 100 MB'],
  ['rozmiar:<1mb', 'Mniejsze niż 1 MB'],
  ['data:dziś', 'Zmienione dziś'],
  ['data:tydzień', 'W ostatnim tygodniu'],
  ['data:2024', 'W roku 2024'],
  ['ext:jpg,png', 'Rozszerzenia'],
]
