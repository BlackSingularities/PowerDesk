import type { Entry, MediaInfo } from './api'
import { kindOf, type Kind } from './util'

/**
 * Filtry dopasowane do zawartości folderu. Pokazujemy tylko te, które coś zmieniają
 * (pasuje więcej niż zero i mniej niż wszystko), więc zestaw sam się zmienia:
 * w folderze ze zdjęciami – formaty, orientacja, daty; z filmami – długość i rozdzielczość;
 * z dokumentami – rodzaje plików.
 */
export interface Facet {
  id: string
  label: string
  group: string
  count: number
}
type Def = { id: string; label: string; group: string; test: (e: Entry, m: MediaInfo | undefined) => boolean }

const DAY = 86_400_000
const DOC_KINDS: [Kind, string][] = [['pdf', 'PDF'], ['doc', 'Teksty'], ['sheet', 'Arkusze'], ['slides', 'Prezentacje'], ['text', 'Notatki']]

function startOfDay() {
  const d = new Date()
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
}

function defsFor(files: Entry[], info: (e: Entry) => MediaInfo | undefined): Def[] {
  const kinds = new Map<Kind, number>()
  for (const e of files) kinds.set(kindOf(e), (kinds.get(kindOf(e)) ?? 0) + 1)
  const n = files.length
  const share = (...ks: Kind[]) => ks.reduce((a, k) => a + (kinds.get(k) ?? 0), 0) / Math.max(1, n)
  const imgs = share('image', 'raw'), vids = share('video'), docs = share('pdf', 'doc', 'sheet', 'slides', 'text')
  const defs: Def[] = []
  const when = (e: Entry) => (kindOf(e) === 'image' || kindOf(e) === 'raw' || kindOf(e) === 'video' ? info(e)?.taken || e.modified : e.modified)

  // Formaty – najczęstsze rozszerzenia (RAW razem, JPG/JPEG razem).
  const fmt = (e: Entry) => (kindOf(e) === 'raw' ? 'RAW' : e.ext === 'jpeg' || e.ext === 'jfif' ? 'JPG' : e.ext.toUpperCase())
  const fmts = new Map<string, number>()
  for (const e of files) if (e.ext) fmts.set(fmt(e), (fmts.get(fmt(e)) ?? 0) + 1)
  ;[...fmts.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 6)
    .forEach(([f]) => defs.push({ id: 'fmt:' + f, label: f, group: 'Format', test: (e) => fmt(e) === f }))

  if (imgs >= 0.3) {
    const ratio = (m?: MediaInfo) => (m?.w && m.h ? m.w / m.h : 0)
    defs.push(
      { id: 'or:land', label: 'Poziome', group: 'Kadr', test: (_, m) => ratio(m) > 1.05 && ratio(m) < 2 },
      { id: 'or:port', label: 'Pionowe', group: 'Kadr', test: (_, m) => ratio(m) > 0 && ratio(m) < 0.95 },
      { id: 'or:sq', label: 'Kwadratowe', group: 'Kadr', test: (_, m) => ratio(m) >= 0.95 && ratio(m) <= 1.05 },
      { id: 'or:pano', label: 'Panoramy', group: 'Kadr', test: (_, m) => ratio(m) >= 2 },
      { id: 'res:big', label: '20 Mpx+', group: 'Rozdzielczość', test: (_, m) => !!m && m.w * m.h >= 20e6 },
    )
  }
  if (vids >= 0.15) {
    defs.push(
      { id: 'dur:short', label: 'Do 1 min', group: 'Długość', test: (e, m) => kindOf(e) === 'video' && !!m?.dur && m.dur < 60 },
      { id: 'dur:mid', label: '1–10 min', group: 'Długość', test: (e, m) => kindOf(e) === 'video' && !!m?.dur && m.dur >= 60 && m.dur < 600 },
      { id: 'dur:long', label: 'Ponad 10 min', group: 'Długość', test: (e, m) => kindOf(e) === 'video' && !!m?.dur && m.dur >= 600 },
      { id: 'vres:4k', label: '4K', group: 'Jakość', test: (e, m) => kindOf(e) === 'video' && !!m && Math.min(m.w, m.h) >= 2000 },
      { id: 'vres:hd', label: 'Full HD', group: 'Jakość', test: (e, m) => kindOf(e) === 'video' && !!m && Math.min(m.w, m.h) >= 1000 && Math.min(m.w, m.h) < 2000 },
    )
  }
  if (docs >= 0.3) {
    for (const [k, label] of DOC_KINDS) defs.push({ id: 'doc:' + k, label, group: 'Rodzaj', test: (e) => kindOf(e) === k })
  }
  if (vids >= 0.15 || share('archive', 'app', 'other') >= 0.3) {
    defs.push({ id: 'size:big', label: 'Duże (100 MB+)', group: 'Rozmiar', test: (e) => e.size >= 100 * 2 ** 20 })
  }

  const today = startOfDay()
  const year = new Date(new Date().getFullYear(), 0, 1).getTime()
  defs.push(
    { id: 'date:today', label: 'Dziś', group: 'Data', test: (e) => when(e) >= today },
    { id: 'date:week', label: '7 dni', group: 'Data', test: (e) => when(e) >= today - 6 * DAY },
    { id: 'date:month', label: '30 dni', group: 'Data', test: (e) => when(e) >= today - 29 * DAY },
    { id: 'date:year', label: 'Ten rok', group: 'Data', test: (e) => when(e) >= year },
    { id: 'date:old', label: 'Starsze', group: 'Data', test: (e) => when(e) < year },
  )
  return defs
}

/** Definicje wg id – testy zależą tylko od id, więc można je trzymać globalnie dla wszystkich kart. */
const registry = new Map<string, Def>()

/** Dostępne filtry dla listy plików (tylko te, które realnie zawężają wynik). */
export function facetsFor(list: Entry[], info: (e: Entry) => MediaInfo | undefined): Facet[] {
  const files = list.filter((e) => !e.dir)
  if (files.length < 4 || files.length > 20000) return []
  const defs = defsFor(files, info)
  for (const d of defs) registry.set(d.id, d)
  const out: Facet[] = []
  for (const d of defs) {
    let c = 0
    for (const e of files) if (d.test(e, info(e))) c++
    if (c > 0 && c < files.length) out.push({ id: d.id, label: d.label, group: d.group, count: c })
  }
  return out
}

/** Filtr wybranych faset: w obrębie grupy – lub, między grupami – i. */
export function facetTest(ids: Iterable<string>, info: (e: Entry) => MediaInfo | undefined): ((e: Entry) => boolean) | null {
  const chosen = [...ids].map((id) => registry.get(id) ?? null).filter((d): d is Def => !!d)
  if (!chosen.length) return null
  const groups = new Map<string, Def[]>()
  for (const d of chosen) groups.set(d.group, [...(groups.get(d.group) ?? []), d])
  return (e) => !e.dir && [...groups.values()].every((g) => g.some((d) => d.test(e, info(e))))
}
