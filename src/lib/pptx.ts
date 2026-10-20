/**
 * Lekki podgląd prezentacji PowerPoint (.pptx): slajdy z polami tekstowymi i obrazami
 * w ich prawdziwych położeniach. Bez animacji, wykresów i SmartArtu – to podgląd, nie odtwarzacz.
 */
import JSZip from 'jszip'

export interface Box { x: number; y: number; w: number; h: number }
export interface SlideText extends Box { kind: 'text'; paras: { text: string; size: number; bold: boolean; color: string | null; align: string }[] }
export interface SlideImage extends Box { kind: 'image'; src: string }
export interface Slide { items: (SlideText | SlideImage)[]; bg: string | null }
export interface Deck { w: number; h: number; slides: Slide[] }

const NS_A = 'http://schemas.openxmlformats.org/drawingml/2006/main'
const NS_P = 'http://schemas.openxmlformats.org/presentationml/2006/main'
const NS_R = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'

const MIME: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', bmp: 'image/bmp', svg: 'image/svg+xml', webp: 'image/webp' }

function parse(xml: string) {
  return new DOMParser().parseFromString(xml, 'application/xml')
}

function box(el: Element): Box | null {
  const off = el.getElementsByTagNameNS(NS_A, 'off')[0]
  const ext = el.getElementsByTagNameNS(NS_A, 'ext')[0]
  if (!off || !ext) return null
  return { x: +off.getAttribute('x')!, y: +off.getAttribute('y')!, w: +ext.getAttribute('cx')!, h: +ext.getAttribute('cy')! }
}

function color(el: Element | undefined): string | null {
  const c = el?.getElementsByTagNameNS(NS_A, 'srgbClr')[0]?.getAttribute('val')
  return c ? '#' + c : null
}

async function rels(zip: JSZip, path: string) {
  const relPath = path.replace(/([^/]+)$/, '_rels/$1.rels')
  const map = new Map<string, string>()
  const x = await zip.file(relPath)?.async('string')
  if (!x) return map
  for (const r of Array.from(parse(x).getElementsByTagName('Relationship'))) {
    const target = r.getAttribute('Target') ?? ''
    // Ścieżki względne do folderu slajdu.
    const base = path.split('/').slice(0, -1)
    for (const part of target.split('/')) {
      if (part === '..') base.pop()
      else if (part !== '.') base.push(part)
    }
    map.set(r.getAttribute('Id') ?? '', base.join('/'))
  }
  return map
}

export async function readDeck(data: ArrayBuffer, maxSlides = 60): Promise<Deck> {
  const zip = await JSZip.loadAsync(data)
  const pres = parse((await zip.file('ppt/presentation.xml')?.async('string')) ?? '')
  const sz = pres.getElementsByTagNameNS(NS_P, 'sldSz')[0]
  const w = +(sz?.getAttribute('cx') ?? 12192000), h = +(sz?.getAttribute('cy') ?? 6858000)

  // Kolejność slajdów z presentation.xml (sldIdLst → relacje).
  const presRels = await rels(zip, 'ppt/presentation.xml')
  const order = Array.from(pres.getElementsByTagNameNS(NS_P, 'sldId')).map((s) => presRels.get(s.getAttributeNS(NS_R, 'id') ?? '') ?? '').filter(Boolean)

  const slides: Slide[] = []
  for (const path of order.slice(0, maxSlides)) {
    const doc = parse((await zip.file(path)?.async('string')) ?? '')
    const r = await rels(zip, path)
    const items: Slide['items'] = []
    const bg = color(doc.getElementsByTagNameNS(NS_P, 'bg')[0])

    for (const sp of Array.from(doc.getElementsByTagNameNS(NS_P, 'sp'))) {
      const b = box(sp)
      const paras = Array.from(sp.getElementsByTagNameNS(NS_A, 'p')).map((p) => {
        const runs = Array.from(p.getElementsByTagNameNS(NS_A, 'r'))
        const rPr = runs[0]?.getElementsByTagNameNS(NS_A, 'rPr')[0]
        return {
          text: runs.map((x) => x.getElementsByTagNameNS(NS_A, 't')[0]?.textContent ?? '').join(''),
          size: +(rPr?.getAttribute('sz') ?? 1800) / 100,
          bold: rPr?.getAttribute('b') === '1',
          color: color(rPr ?? undefined),
          align: p.getElementsByTagNameNS(NS_A, 'pPr')[0]?.getAttribute('algn') ?? 'l',
        }
      })
      if (!paras.some((p) => p.text.trim())) continue
      // Pola z układu slajdu (tytuł, treść) nie mają własnej pozycji – dajemy rozsądne domyślne.
      const isTitle = !!sp.getElementsByTagNameNS(NS_P, 'ph')[0]?.getAttribute('type')?.includes('itle')
      const fb: Box = isTitle ? { x: w * 0.06, y: h * 0.05, w: w * 0.88, h: h * 0.18 } : { x: w * 0.06, y: h * 0.26, w: w * 0.88, h: h * 0.66 }
      items.push({ kind: 'text', ...(b ?? fb), paras })
    }
    for (const pic of Array.from(doc.getElementsByTagNameNS(NS_P, 'pic'))) {
      const b = box(pic)
      const id = pic.getElementsByTagNameNS(NS_A, 'blip')[0]?.getAttributeNS(NS_R, 'embed')
      const file = id ? r.get(id) : null
      const ext = file?.split('.').pop()?.toLowerCase() ?? ''
      if (!b || !file || !MIME[ext]) continue
      const b64 = await zip.file(file)?.async('base64')
      if (b64) items.push({ kind: 'image', ...b, src: `data:${MIME[ext]};base64,${b64}` })
    }
    slides.push({ items, bg })
  }
  return { w, h, slides }
}
