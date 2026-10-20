<script lang="ts" module>
  import type { Entry } from '../lib/api'
  import { kindOf } from '../lib/util'

  const MD = new Set(['md', 'markdown', 'mdx'])
  const SHEET = new Set(['xlsx', 'xlsm', 'xls', 'ods', 'csv', 'tsv'])
  const WORD = new Set(['docx', 'docm', 'dotx'])
  const SLIDES = new Set(['pptx', 'ppsx', 'pptm'])

  /** Czy ten plik ma bogaty podgląd dokumentu (kod, Markdown, Word, arkusz). */
  export function hasDocPreview(e: Entry) {
    if (e.dir || e.cloud) return false
    const k = kindOf(e)
    return MD.has(e.ext) || SHEET.has(e.ext) || WORD.has(e.ext) || SLIDES.has(e.ext) || k === 'code' || k === 'text'
  }
</script>

<script lang="ts">
  import { api, fileUrl } from '../lib/api'
  import { size } from '../lib/util'
  import type { Deck } from '../lib/pptx'

  let { entry, compact = false }: { entry: Entry; compact?: boolean } = $props()

  type View =
    | { kind: 'html'; html: string; cls: string }
    | { kind: 'code'; html: string; lang: string; lines: number; truncated: boolean }
    | { kind: 'sheet'; sheets: { name: string; html: string }[] }
    | { kind: 'deck'; deck: Deck }
    | { kind: 'error'; text: string }
  let view = $state<View | null>(null)
  let sheet = $state(0)

  /** Usuwa skrypty i obsługę zdarzeń z HTML-a pochodzącego z pliku. */
  async function clean(html: string) {
    const { default: DOMPurify } = await import('dompurify')
    return DOMPurify.sanitize(html, { FORBID_TAGS: ['style', 'iframe', 'form', 'object', 'embed'], FORBID_ATTR: ['style'] })
  }
  async function bytes(path: string, limit: number) {
    const r = await fetch(fileUrl(path))
    if (!r.ok) throw new Error('Nie udało się odczytać pliku')
    const b = await r.arrayBuffer()
    if (b.byteLength > limit) throw new Error(`Plik jest za duży na podgląd (${size(b.byteLength)})`)
    return b
  }

  const LANG: Record<string, string> = {
    js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'javascript', ts: 'typescript', tsx: 'typescript', py: 'python', rs: 'rust',
    kt: 'kotlin', kts: 'kotlin', cs: 'csharp', h: 'c', hpp: 'cpp', cc: 'cpp', rb: 'ruby', sh: 'bash', ps1: 'powershell', bat: 'dos', cmd: 'dos',
    yml: 'yaml', htm: 'xml', html: 'xml', svelte: 'xml', vue: 'xml', svg: 'xml', toml: 'ini', cfg: 'ini', conf: 'ini', properties: 'properties',
    gradle: 'gradle', log: 'plaintext', txt: 'plaintext', nfo: 'plaintext', srt: 'plaintext', vtt: 'plaintext', reg: 'ini',
  }

  async function render(e: Entry) {
    if (MD.has(e.ext)) {
      const t = await api.readText(e.path, 1_000_000)
      const { marked } = await import('marked')
      return { kind: 'html', html: await clean(await marked.parse(t.text)), cls: 'md' } as View
    }
    if (WORD.has(e.ext)) {
      const mammoth = await import('mammoth')
      const r = await mammoth.convertToHtml({ arrayBuffer: await bytes(e.path, 40 * 2 ** 20) })
      return { kind: 'html', html: await clean(r.value), cls: 'doc' } as View
    }
    if (SLIDES.has(e.ext)) {
      const { readDeck } = await import('../lib/pptx')
      return { kind: 'deck', deck: await readDeck(await bytes(e.path, 200 * 2 ** 20), compact ? 20 : 80) } as View
    }
    if (SHEET.has(e.ext)) {
      const XLSX = await import('xlsx')
      const wb = XLSX.read(await bytes(e.path, 30 * 2 ** 20), { type: 'array', sheetRows: 1000, dense: true })
      const sheets = await Promise.all(wb.SheetNames.slice(0, 30).map(async (n) => ({ name: n, html: await clean(XLSX.utils.sheet_to_html(wb.Sheets[n], { header: '', footer: '' })) })))
      return { kind: 'sheet', sheets } as View
    }
    // Kod i tekst – z kolorowaniem składni.
    const t = await api.readText(e.path, 600_000)
    const hljs = (await import('highlight.js/lib/common')).default
    const lang = LANG[e.ext] ?? e.ext
    const html = hljs.getLanguage(lang) && lang !== 'plaintext' ? hljs.highlight(t.text, { language: lang, ignoreIllegals: true }).value : hljs.highlightAuto(t.text.slice(0, 200_000)).value
    return { kind: 'code', html, lang: hljs.getLanguage(lang)?.name ?? '', lines: t.text.split('\n').length, truncated: t.truncated } as View
  }

  $effect(() => {
    const e = entry
    view = null
    sheet = 0
    render(e)
      .then((v) => entry.path === e.path && (view = v))
      .catch((err) => entry.path === e.path && (view = { kind: 'error', text: String(err?.message ?? err) }))
  })
</script>

<div class="doc" class:compact>
  {#if !view}
    <p class="faint pad">Wczytywanie podglądu…</p>
  {:else if view.kind === 'error'}
    <p class="faint pad">{view.text}</p>
  {:else if view.kind === 'html'}
    <article class={view.cls}>{@html view.html}</article>
  {:else if view.kind === 'code'}
    <div class="codehead faint">{view.lang || 'Tekst'} · {view.lines} wierszy{view.truncated ? ' · pokazano początek pliku' : ''}</div>
    <pre class="code"><code class="hljs">{@html view.html}</code></pre>
  {:else if view.kind === 'deck'}
    {@const d = view.deck}
    <div class="deck">
      {#each d.slides as s, i (i)}
        <figure class="slide" style:aspect-ratio="{d.w} / {d.h}" style:background={s.bg ?? '#fff'} style:container-type="inline-size">
          {#each s.items.filter((x) => x.kind === 'image') as it, j (j)}
            {#if it.kind === 'image'}<img src={it.src} alt="" style:left="{(it.x / d.w) * 100}%" style:top="{(it.y / d.h) * 100}%" style:width="{(it.w / d.w) * 100}%" style:height="{(it.h / d.h) * 100}%" />{/if}
          {/each}
          {#each s.items as it, j (j)}
            {#if it.kind === 'text'}
              <div class="tb" style:left="{(it.x / d.w) * 100}%" style:top="{(it.y / d.h) * 100}%" style:width="{(it.w / d.w) * 100}%" style:height="{(it.h / d.h) * 100}%">
                {#each it.paras as p, k (k)}
                  <p style:font-size="{(p.size / (d.w / 12700)) * 100}cqw" style:font-weight={p.bold ? 700 : 400} style:color={p.color ?? '#222'} style:text-align={p.align === 'ctr' ? 'center' : p.align === 'r' ? 'right' : 'left'}>{p.text || '\u00a0'}</p>
                {/each}
              </div>
            {/if}
          {/each}
          <figcaption>{i + 1}</figcaption>
        </figure>
      {:else}
        <p class="faint pad">Prezentacja nie ma slajdów</p>
      {/each}
    </div>
  {:else}
    {#if view.sheets.length > 1}
      <div class="tabs" role="tablist">
        {#each view.sheets as s, i (s.name)}
          <button role="tab" aria-selected={sheet === i} class:on={sheet === i} onclick={() => (sheet = i)}>{s.name}</button>
        {/each}
      </div>
    {/if}
    <div class="sheet">{@html view.sheets[sheet]?.html ?? ''}</div>
  {/if}
</div>

<style>
  .doc {
    width: 100%;
    height: 100%;
    overflow: auto;
    display: flex;
    flex-direction: column;
    user-select: text;
  }
  .pad {
    padding: 20px;
  }
  article {
    max-width: 860px;
    width: 100%;
    margin: 0 auto;
    padding: 32px 40px 60px;
    line-height: 1.6;
    font-size: 14px;
    color: var(--text);
  }
  .compact article {
    padding: 12px 14px;
    font-size: 13px;
  }
  article :global(img) {
    max-width: 100%;
  }
  article :global(h1),
  article :global(h2),
  article :global(h3) {
    line-height: 1.25;
    margin: 1.4em 0 0.5em;
  }
  article :global(a) {
    color: var(--info);
  }
  article :global(code) {
    font-family: var(--mono);
    font-size: 0.9em;
    background: var(--high);
    padding: 1px 5px;
    border-radius: var(--r-xs);
  }
  article :global(pre) {
    background: var(--high);
    padding: 12px 14px;
    border-radius: var(--r);
    overflow: auto;
  }
  article :global(pre code) {
    background: none;
    padding: 0;
  }
  article :global(table) {
    border-collapse: collapse;
  }
  article :global(td),
  article :global(th) {
    border: 1px solid var(--line-strong);
    padding: 4px 8px;
  }
  article :global(blockquote) {
    margin: 0;
    padding: 4px 14px;
    background: var(--high);
    border-radius: var(--r-s);
    color: var(--dim);
  }
  .codehead {
    flex: none;
    padding: 6px 14px;
    font-size: 11.5px;
  }
  .code {
    margin: 0;
    padding: 4px 14px 24px;
    font: 12.5px/1.55 var(--mono);
    tab-size: 4;
    white-space: pre;
  }
  .compact .code {
    font-size: 11.5px;
  }
  /* Kolory składni – z tokenów motywu, działają w jasnym i ciemnym. */
  .code :global(.hljs-keyword),
  .code :global(.hljs-selector-tag),
  .code :global(.hljs-built_in) {
    color: #c678dd;
  }
  .code :global(.hljs-string),
  .code :global(.hljs-attr),
  .code :global(.hljs-template-tag) {
    color: #98c379;
  }
  .code :global(.hljs-number),
  .code :global(.hljs-literal) {
    color: #d19a66;
  }
  .code :global(.hljs-comment) {
    color: var(--faint);
    font-style: italic;
  }
  .code :global(.hljs-title),
  .code :global(.hljs-function),
  .code :global(.hljs-name) {
    color: #61afef;
  }
  .code :global(.hljs-type),
  .code :global(.hljs-class) {
    color: #e5c07b;
  }
  :global([data-theme='light']) .code :global(.hljs-keyword) {
    color: #a626a4;
  }
  :global([data-theme='light']) .code :global(.hljs-string),
  :global([data-theme='light']) .code :global(.hljs-attr) {
    color: #3f8f2f;
  }
  :global([data-theme='light']) .code :global(.hljs-number) {
    color: #b45f06;
  }
  :global([data-theme='light']) .code :global(.hljs-title),
  :global([data-theme='light']) .code :global(.hljs-function) {
    color: #1f63c4;
  }
  .deck {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 18px;
    padding: 20px;
  }
  .compact .deck {
    gap: 10px;
    padding: 10px;
  }
  .slide {
    position: relative;
    width: min(100%, 960px);
    margin: 0;
    border-radius: var(--r-s);
    overflow: hidden;
    box-shadow: 0 4px 18px var(--shadow);
    font-family: Calibri, 'Segoe UI', sans-serif;
  }
  .slide img,
  .tb {
    position: absolute;
  }
  .slide img {
    object-fit: contain;
  }
  .tb {
    overflow: hidden;
  }
  .tb p {
    margin: 0;
    line-height: 1.15;
    white-space: pre-wrap;
  }
  figcaption {
    position: absolute;
    right: 8px;
    bottom: 6px;
    font-size: 11px;
    color: rgba(0, 0, 0, 0.45);
  }
  .tabs {
    flex: none;
    display: flex;
    gap: 4px;
    padding: 8px 10px 0;
    overflow-x: auto;
  }
  .tabs button {
    padding: 4px 12px;
    border-radius: var(--r-s);
    font-size: 12px;
    color: var(--dim);
    white-space: nowrap;
  }
  .tabs button.on {
    background: var(--accent);
    color: var(--on-accent);
  }
  .sheet {
    padding: 10px;
    overflow: auto;
  }
  .sheet :global(table) {
    border-collapse: collapse;
    font-size: 12.5px;
    font-variant-numeric: tabular-nums;
  }
  .sheet :global(td) {
    border: 1px solid var(--line);
    padding: 3px 8px;
    white-space: nowrap;
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sheet :global(tr:first-child td) {
    font-weight: 600;
    background: var(--high);
  }
</style>
