// Awaryjne miniatury PDF, gdy Windows nie ma zainstalowanej obsługi miniatur PDF.
// Renderowane tylko w pamięci (nic nie jest zapisywane na dysk), z limitem równoległości.
import { fileUrl } from './api'

type PdfJs = typeof import('pdfjs-dist')
let lib: Promise<PdfJs> | null = null
const cache = new Map<string, string>()
const order: string[] = []
let active = 0
const queue: (() => void)[] = []

function pdfjs() {
  if (!lib) {
    lib = import('pdfjs-dist').then(async (m) => {
      const worker = await import('pdfjs-dist/build/pdf.worker.min.mjs?url')
      m.GlobalWorkerOptions.workerSrc = worker.default
      return m
    })
  }
  return lib
}

async function slot<T>(fn: () => Promise<T>): Promise<T> {
  if (active >= 2) await new Promise<void>((r) => queue.push(r))
  active++
  try {
    return await fn()
  } finally {
    active--
    queue.shift()?.()
  }
}

export async function pdfThumb(path: string, px: number, stamp: number): Promise<string | null> {
  const key = `${path}|${px}|${stamp}`
  const hit = cache.get(key)
  if (hit) return hit
  return slot(async () => {
    try {
      const m = await pdfjs()
      const doc = await m.getDocument({ url: fileUrl(path), disableAutoFetch: true, disableStream: false, isEvalSupported: false }).promise
      const page = await doc.getPage(1)
      const vp0 = page.getViewport({ scale: 1 })
      const scale = (px * (window.devicePixelRatio || 1)) / Math.max(vp0.width, vp0.height)
      const vp = page.getViewport({ scale })
      const canvas = document.createElement('canvas')
      canvas.width = Math.ceil(vp.width)
      canvas.height = Math.ceil(vp.height)
      const ctx = canvas.getContext('2d')!
      ctx.fillStyle = '#fff'
      ctx.fillRect(0, 0, canvas.width, canvas.height)
      await page.render({ canvasContext: ctx, viewport: vp }).promise
      doc.destroy()
      const blob: Blob | null = await new Promise((r) => canvas.toBlob(r, 'image/jpeg', 0.85))
      if (!blob) return null
      const url = URL.createObjectURL(blob)
      cache.set(key, url)
      order.push(key)
      if (order.length > 400) {
        const old = order.shift()!
        URL.revokeObjectURL(cache.get(old)!)
        cache.delete(old)
      }
      return url
    } catch {
      return null
    }
  })
}
