<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import Icon from './Icon.svelte'
  import { app, navigate, newTab, toast, confirm, SPACE, type Tab } from '../lib/state.svelte'
  import { api, on } from '../lib/api'
  import { size, basename, date, kindOf } from '../lib/util'

  let { tab }: { tab: Tab } = $props()

  type Node = { name: string; path: string; size: number; files: number; dir: boolean; children: Node[] }
  type Junk = { path: string; size: number; kind: 'empty' | 'junk' | 'temp'; reason: string; modified: number }

  const root = $derived(tab.path.slice(SPACE.length + 1))
  let mode = $state<'map' | 'clean'>('map')
  let tree = $state<Node | null>(null)
  let stack = $state<Node[]>([])
  let busy = $state(false)
  let prog = $state({ files: 0, bytes: 0, current: '' })
  let err = $state('')
  let w = $state(800)
  let h = $state(500)
  let hover = $state<Node | null>(null)

  const cur = $derived(stack.at(-1) ?? tree)

  async function scan() {
    busy = true
    err = ''
    tree = null
    stack = []
    prog = { files: 0, bytes: 0, current: '' }
    const off = await on<typeof prog>('space-progress', (p) => (prog = p))
    try {
      tree = await invoke<Node>('space_scan', { path: root })
    } catch (e) {
      err = String(e)
    }
    off()
    busy = false
  }
  $effect(() => {
    root
    if (mode === 'map' && !tree && !busy) scan()
  })

  // ---------- Squarified treemap ----------
  type Rect = { n: Node; x: number; y: number; w: number; h: number }
  function squarify(nodes: Node[], x: number, y: number, W: number, H: number): Rect[] {
    const items = nodes.filter((n) => n.size > 0)
    const total = items.reduce((a, n) => a + n.size, 0)
    if (!total || W <= 0 || H <= 0) return []
    const scale = (W * H) / total
    const out: Rect[] = []
    let row: Node[] = []
    let rx = x, ry = y, rw = W, rh = H
    const worst = (r: Node[], side: number) => {
      const s = r.reduce((a, n) => a + n.size * scale, 0)
      const mx = Math.max(...r.map((n) => n.size * scale)), mn = Math.min(...r.map((n) => n.size * scale))
      return Math.max((side * side * mx) / (s * s), (s * s) / (side * side * mn))
    }
    const layout = (r: Node[]) => {
      const s = r.reduce((a, n) => a + n.size * scale, 0)
      if (rw >= rh) {
        const cw = s / rh
        let yy = ry
        for (const n of r) {
          const hh = (n.size * scale) / cw
          out.push({ n, x: rx, y: yy, w: cw, h: hh })
          yy += hh
        }
        rx += cw
        rw -= cw
      } else {
        const ch = s / rw
        let xx = rx
        for (const n of r) {
          const ww = (n.size * scale) / ch
          out.push({ n, x: xx, y: ry, w: ww, h: ch })
          xx += ww
        }
        ry += ch
        rh -= ch
      }
    }
    for (const n of items) {
      const side = Math.min(rw, rh)
      if (!row.length || worst([...row, n], side) <= worst(row, side)) row.push(n)
      else {
        layout(row)
        row = [n]
      }
    }
    if (row.length) layout(row)
    return out
  }
  const rects = $derived(cur ? squarify(cur.children, 0, 0, w, h) : [])

  const PALETTE: Record<string, string> = { image: '#e8a33d', raw: '#d48a2a', video: '#d9534f', audio: '#9b59b6', archive: '#7f8c8d', app: '#3498db', doc: '#2e86de', pdf: '#c0392b', sheet: '#27ae60', code: '#16a085' }
  function colorOf(n: Node, i: number) {
    if (n.dir) return `hsl(${(i * 47) % 360} 32% ${38 - Math.min(i, 8)}%)`
    const ext = n.name.split('.').pop()?.toLowerCase() ?? ''
    return PALETTE[kindOf({ dir: false, ext })] ?? '#55606e'
  }

  function open(n: Node) {
    if (n.dir && n.children.length) stack = [...stack, n]
    else if (n.dir) navigate(tab, n.path)
  }
  function tileMenu(e: MouseEvent, n: Node) {
    e.preventDefault()
    if (!n.path) return
    app.menu = {
      x: e.clientX, y: e.clientY,
      items: [
        ...(n.dir ? [{ label: 'Pokaż szczegóły', icon: 'grid', run: () => open(n) }] : []),
        { label: n.dir ? 'Otwórz folder' : 'Pokaż w folderze', icon: 'folder', run: () => navigate(tab, n.dir ? n.path : n.path.slice(0, n.path.lastIndexOf('\\')) || n.path) },
        { label: 'Otwórz w nowej karcie', icon: 'plus', run: () => newTab(n.dir ? n.path : n.path.slice(0, n.path.lastIndexOf('\\')), false, tab.id) },
        { sep: true },
        { label: 'Do Kosza', icon: 'trash', danger: true, run: () => trashPaths([n.path], `„${n.name}” (${size(n.size)})`) },
      ],
    }
  }

  async function trashPaths(paths: string[], what: string) {
    if (!(await confirm('Przenieść do Kosza?', `${what} trafi do Kosza – można je stamtąd przywrócić.`, 'Do Kosza'))) return
    await api.startOp('trash', paths)
    toast('Przeniesiono do Kosza', 'ok')
  }

  // ---------- Porządki ----------
  let junk = $state<Junk[] | null>(null)
  let withTemp = $state(false)
  let picked = $state(new Set<string>())
  async function cleanScan() {
    busy = true
    junk = null
    try {
      junk = await invoke<Junk[]>('cleanup_scan', { path: root, temp: withTemp })
      picked = new Set(junk.map((j) => j.path))
    } catch (e) {
      err = String(e)
    }
    busy = false
  }
  const groups = $derived.by(() => {
    const g: Record<string, Junk[]> = { empty: [], junk: [], temp: [] }
    for (const j of junk ?? []) g[j.kind].push(j)
    return [
      { id: 'empty', title: 'Puste foldery', hint: 'Nie zawierają żadnych plików, także w podfolderach', list: g.empty },
      { id: 'junk', title: 'Śmieci', hint: 'Pamięć miniatur, pozostałości po macOS i Office, stare pliki tymczasowe i nieukończone pobierania', list: g.junk },
      { id: 'temp', title: 'Folder TEMP', hint: 'Pliki tymczasowe programów nieużywane od tygodnia', list: g.temp },
    ].filter((x) => x.list.length)
  })
  const pickedSize = $derived((junk ?? []).filter((j) => picked.has(j.path)).reduce((a, j) => a + j.size, 0))
  function togglePick(p: string) {
    const s = new Set(picked)
    if (s.has(p)) s.delete(p)
    else s.add(p)
    picked = s
  }
  function toggleGroup(list: Junk[]) {
    const s = new Set(picked)
    const all = list.every((j) => s.has(j.path))
    for (const j of list) all ? s.delete(j.path) : s.add(j.path)
    picked = s
  }
  async function clean() {
    const paths = [...picked]
    if (!paths.length) return
    await trashPaths(paths, `${paths.length} elementów (${size(pickedSize)})`)
    junk = (junk ?? []).filter((j) => !picked.has(j.path))
    picked = new Set()
  }
</script>

<div class="space">
  <header class="row">
    <div class="seg">
      <button class:on={mode === 'map'} onclick={() => (mode = 'map')}><Icon name="grid" size={15} />Zajętość</button>
      <button class:on={mode === 'clean'} onclick={() => { mode = 'clean'; if (!junk && !busy) cleanScan() }}><Icon name="trash" size={15} />Porządki</button>
    </div>
    <span class="grow ellipsis path" title={root}>{root}</span>
    {#if busy}
      <span class="faint small">{prog.files ? `${prog.files.toLocaleString('pl')} plików · ${size(prog.bytes)}` : 'Skanuję…'}</span>
      <button class="btn ghost sm" onclick={() => invoke('space_cancel')}>Przerwij</button>
    {:else}
      <button class="btn sm" onclick={() => (mode === 'map' ? scan() : cleanScan())}><Icon name="refresh" size={14} />Skanuj ponownie</button>
    {/if}
  </header>

  {#if err}<p class="err">{err}</p>{/if}

  {#if mode === 'map'}
    {#if tree && cur}
      <div class="crumbs row">
        <button class="crumb" onclick={() => (stack = [])}>{basename(tree.path) || tree.path}</button>
        {#each stack as s, i (s.path)}
          <Icon name="chevron" size={12} /><button class="crumb" onclick={() => (stack = stack.slice(0, i + 1))}>{s.name}</button>
        {/each}
        <span class="grow"></span>
        <span class="faint small">{size(cur.size)} · {cur.files.toLocaleString('pl')} plików</span>
      </div>
      <div class="body">
        <div class="map" bind:clientWidth={w} bind:clientHeight={h} role="img" aria-label="Mapa zajętości">
          {#each rects as r, i (r.n.path || r.n.name)}
            <button
              class="tile"
              class:dir={r.n.dir}
              style:left="{r.x}px" style:top="{r.y}px" style:width="{r.w}px" style:height="{r.h}px"
              style:background={colorOf(r.n, i)}
              onclick={() => open(r.n)}
              oncontextmenu={(e) => tileMenu(e, r.n)}
              onmouseenter={() => (hover = r.n)}
              onmouseleave={() => (hover = null)}
              title="{r.n.name}&#10;{size(r.n.size)}{r.n.dir ? ` · ${r.n.files} plików` : ''}"
            >
              {#if r.w > 70 && r.h > 34}<span class="tn ellipsis">{r.n.name}</span><span class="ts">{size(r.n.size)}</span>{/if}
            </button>
          {/each}
        </div>
        <aside class="top">
          <h3>Największe</h3>
          {#each cur.children.slice(0, 40) as n, i (n.path || n.name)}
            <button class="trow" class:hl={hover === n} onclick={() => open(n)} oncontextmenu={(e) => tileMenu(e, n)}>
              <span class="dot2" style:background={colorOf(n, i)}></span>
              <span class="ellipsis grow">{n.name}</span>
              <span class="faint">{size(n.size)}</span>
              <span class="pct" style:width="{Math.max(2, (n.size / (cur.size || 1)) * 60)}px"></span>
            </button>
          {/each}
        </aside>
      </div>
    {:else if busy}
      <div class="wait"><span class="dot busy"></span><p>Liczę rozmiary… {prog.current}</p></div>
    {/if}
  {:else}
    <div class="clean">
      <div class="opts row">
        <label class="row"><input type="checkbox" bind:checked={withTemp} onchange={cleanScan} />Uwzględnij folder TEMP systemu</label>
        <span class="grow"></span>
        <span class="faint small">Wszystko trafia do Kosza – można przywrócić. Foldery programów, systemowe, ukryte i repozytoria są pomijane.</span>
      </div>
      {#if busy}
        <div class="wait"><span class="dot busy"></span><p>Szukam…</p></div>
      {:else if junk && !junk.length}
        <div class="wait"><Icon name="check" size={26} /><p>Czysto – nie znaleziono nic do usunięcia.</p></div>
      {:else if junk}
        <div class="glist">
          {#each groups as g (g.id)}
            <section>
              <h3 class="row"><input type="checkbox" checked={g.list.every((j) => picked.has(j.path))} onchange={() => toggleGroup(g.list)} aria-label="Zaznacz grupę" /><span>{g.title}</span><span class="faint">{g.list.length} · {size(g.list.reduce((a, j) => a + j.size, 0))}</span></h3>
              <p class="faint small">{g.hint}</p>
              {#each g.list.slice(0, 500) as j (j.path)}
                <label class="jrow">
                  <input type="checkbox" checked={picked.has(j.path)} onchange={() => togglePick(j.path)} />
                  <span class="ellipsis grow" title={j.path}>{j.path}</span>
                  <span class="faint small">{j.reason}</span>
                  <span class="faint sz">{j.size ? size(j.size) : ''}</span>
                  <span class="faint sz">{date(j.modified)}</span>
                </label>
              {/each}
            </section>
          {/each}
        </div>
        <footer class="row">
          <span class="grow">Zaznaczono {picked.size} · {size(pickedSize)}</span>
          <button class="btn primary" disabled={!picked.size} onclick={clean}><Icon name="trash" size={15} />Przenieś do Kosza</button>
        </footer>
      {/if}
    </div>
  {/if}
</div>

<style>
  .space {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 14px 12px;
  }
  header {
    gap: 10px;
  }
  .path {
    color: var(--dim);
    font-size: 12.5px;
  }
  .small {
    font-size: 12px;
  }
  .btn.sm {
    height: 28px;
    padding: 0 10px;
    font-size: 12px;
  }
  .seg {
    display: flex;
    background: var(--high);
    border-radius: var(--r);
    padding: 2px;
  }
  .seg button {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 12px;
    border-radius: var(--r-s);
    color: var(--dim);
    font-size: 12.5px;
  }
  .seg button.on {
    background: var(--accent);
    color: var(--on-accent);
  }
  .err {
    color: var(--rec);
    margin: 0;
  }
  .crumbs {
    gap: 4px;
    font-size: 12.5px;
    color: var(--faint);
  }
  .crumb {
    padding: 2px 6px;
    border-radius: var(--r-xs);
    color: var(--dim);
  }
  .crumb:hover {
    background: var(--hover);
    color: var(--text);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 12px;
  }
  .map {
    position: relative;
    flex: 1;
    min-width: 0;
    border-radius: var(--r);
    overflow: hidden;
    background: var(--panel);
  }
  .tile {
    position: absolute;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    padding: 4px 6px;
    text-align: left;
    color: #fff;
    outline: 1px solid rgba(0, 0, 0, 0.35);
    outline-offset: -1px;
    transition: filter 0.1s;
    overflow: hidden;
  }
  .tile:hover {
    filter: brightness(1.18);
    z-index: 1;
  }
  .tn {
    font-size: 12px;
    font-weight: 600;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.6);
  }
  .ts {
    font-size: 11px;
    opacity: 0.85;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.6);
  }
  .top {
    width: 300px;
    flex: none;
    overflow-y: auto;
  }
  h3 {
    margin: 4px 0 8px;
    font-size: 13px;
    font-weight: 600;
  }
  .trow {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 28px;
    padding: 0 8px;
    border-radius: var(--r-s);
    font-size: 12.5px;
    color: var(--dim);
    text-align: left;
  }
  .trow:hover,
  .trow.hl {
    background: var(--hover);
    color: var(--text);
  }
  .dot2 {
    width: 10px;
    height: 10px;
    border-radius: var(--r-xs);
    flex: none;
  }
  .pct {
    height: 4px;
    border-radius: 2px;
    background: var(--accent);
    opacity: 0.6;
    flex: none;
  }
  .wait {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--faint);
    text-align: center;
  }
  .clean {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .opts {
    font-size: 12.5px;
  }
  .glist {
    flex: 1;
    overflow-y: auto;
  }
  .glist section {
    margin-bottom: 16px;
  }
  .glist h3 {
    gap: 8px;
  }
  .glist p {
    margin: -4px 0 6px 24px;
  }
  .jrow {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 28px;
    padding: 0 6px;
    border-radius: var(--r-s);
    font-size: 12.5px;
  }
  .jrow:hover {
    background: var(--hover);
  }
  .sz {
    width: 90px;
    text-align: right;
    font-size: 12px;
    flex: none;
  }
  input[type='checkbox'] {
    accent-color: var(--accent);
    padding: 0;
  }
  footer {
    padding-top: 8px;
    border-top: 1px solid var(--line);
  }
</style>
