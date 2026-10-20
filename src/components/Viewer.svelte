<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import Icon from './Icon.svelte'
  import Thumb from './Thumb.svelte'
  import DocPreview, { hasDocPreview } from './DocPreview.svelte'
  import { app, toast } from '../lib/state.svelte'
  import { api, fileUrl, thumbUrl, type Prop, type TextPreview } from '../lib/api'
  import { kindOf, size, date, WEB_IMAGE, WEB_VIDEO, WEB_AUDIO } from '../lib/util'

  const v = $derived(app.viewer!)
  const e = $derived(v.list[v.index])
  const k = $derived(e ? kindOf(e) : 'other')

  let scale = $state(1)
  let fit = $state(true)
  let tx = $state(0)
  let ty = $state(0)
  let rot = $state(0)
  let natural = $state({ w: 0, h: 0 })
  let stageEl = $state<HTMLDivElement>()
  let videoEl = $state<HTMLVideoElement>()
  let showInfo = $state(false)
  let props = $state<Prop[]>([])
  let text = $state<TextPreview | null>(null)
  let slideshow = $state(false)
  let chrome = $state(true)
  let hideTimer: ReturnType<typeof setTimeout> | undefined
  let loadedSrc = $state('')
  let hiRes = $state(false)

  const src = $derived(
    !e ? '' : k === 'image' && WEB_IMAGE.has(e.ext) ? fileUrl(e.path) : thumbUrl(e.path, 2560, 't') + `&m=${e.modified}`,
  )

  $effect(() => {
    e?.path
    scale = 1
    fit = true
    tx = 0
    ty = 0
    rot = 0
    text = null
    props = []
    hiRes = false
    if (!e) return
    const path = e.path
    if (showInfo) api.shellProps(path).then((p) => e?.path === path && (props = p))
    if ((k === 'text' || k === 'code') && !e.cloud && !hasDocPreview(e)) api.readText(path, 400_000).then((t) => e?.path === path && (text = t))
    // Sąsiednie zdjęcia ładujemy z wyprzedzeniem – przełączanie jest natychmiastowe.
    for (const d of [1, -1, 2]) {
      const n = v.list[v.index + d]
      if (n && kindOf(n) === 'image' && WEB_IMAGE.has(n.ext)) new Image().src = fileUrl(n.path)
    }
  })
  $effect(() => {
    if (showInfo && e && !props.length) {
      const path = e.path
      api.shellProps(path).then((p) => e?.path === path && (props = p))
    }
  })

  $effect(() => {
    if (!slideshow) return
    const t = setInterval(() => go(1, true), 4000)
    return () => clearInterval(t)
  })

  function go(d: number, wrap = false) {
    let i = v.index + d
    if (wrap) i = (i + v.list.length) % v.list.length
    if (i < 0 || i >= v.list.length) return
    app.viewer = { list: v.list, index: i }
  }

  // Kółko nad taśmą przewija zdjęcia (w dół / w prawo – następne). Gładzik daje drobne kroki, więc je sumujemy.
  let stripAcc = 0
  function stripWheel(e: WheelEvent) {
    e.preventDefault()
    e.stopPropagation()
    const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY
    stripAcc += e.deltaMode === 1 ? d * 40 : d
    if (Math.abs(stripAcc) < 40) return
    go(stripAcc > 0 ? 1 : -1)
    stripAcc = 0
  }

  function close() {
    slideshow = false
    getCurrentWindow().isFullscreen().then((f) => {
      if (f) getCurrentWindow().setFullscreen(false)
    })
    app.viewer = null
  }

  function fitScale() {
    if (!stageEl || !natural.w) return 1
    const r = stageEl.getBoundingClientRect()
    const rotated = rot % 180 !== 0
    const w = rotated ? natural.h : natural.w
    const h = rotated ? natural.w : natural.h
    return Math.min(r.width / w, r.height / h, 1)
  }
  const effScale = $derived(fit ? fitScale() : scale)

  function zoomAt(factor: number, cx?: number, cy?: number) {
    if (!stageEl) return
    const cur = effScale
    const next = Math.max(0.05, Math.min(20, cur * factor))
    const r = stageEl.getBoundingClientRect()
    const px = (cx ?? r.left + r.width / 2) - (r.left + r.width / 2)
    const py = (cy ?? r.top + r.height / 2) - (r.top + r.height / 2)
    // Punkt pod kursorem zostaje na miejscu.
    tx = px - ((px - tx) * next) / cur
    ty = py - ((py - ty) * next) / cur
    scale = next
    fit = false
    if (next > 1.5) hiRes = true
  }

  function onWheel(ev: WheelEvent) {
    if (k !== 'image' && k !== 'raw') return
    ev.preventDefault()
    if (ev.ctrlKey || !ev.shiftKey) zoomAt(ev.deltaY < 0 ? 1.15 : 1 / 1.15, ev.clientX, ev.clientY)
  }

  let drag: { x: number; y: number; tx: number; ty: number } | null = null
  function down(ev: PointerEvent) {
    if (ev.button !== 0 || fit) return
    drag = { x: ev.clientX, y: ev.clientY, tx, ty }
    ;(ev.currentTarget as HTMLElement).setPointerCapture(ev.pointerId)
  }
  function move(ev: PointerEvent) {
    poke()
    if (!drag) return
    tx = drag.tx + ev.clientX - drag.x
    ty = drag.ty + ev.clientY - drag.y
  }

  function toggleFit(ev?: MouseEvent) {
    if (fit) {
      if (ev) zoomAt(1 / effScale, ev.clientX, ev.clientY)
      else {
        scale = 1
        fit = false
      }
      hiRes = true
    } else {
      fit = true
      tx = ty = 0
    }
  }

  async function del() {
    if (!e) return
    const path = e.path
    await api.startOp('trash', [path])
    const list = v.list.filter((x) => x.path !== path)
    if (!list.length) return close()
    app.viewer = { list, index: Math.min(v.index, list.length - 1) }
    toast('Przeniesiono do Kosza', 'ok')
  }

  function onKey(ev: KeyboardEvent) {
    if (!app.viewer || app.dialog) return
    const key = ev.key
    if (key === 'Escape') close()
    else if (key === 'ArrowRight' || key === 'PageDown') go(1)
    else if (key === 'ArrowLeft' || key === 'PageUp') go(-1)
    else if (key === 'Home') app.viewer = { list: v.list, index: 0 }
    else if (key === 'End') app.viewer = { list: v.list, index: v.list.length - 1 }
    else if (key === ' ') {
      if (videoEl) videoEl.paused ? videoEl.play() : videoEl.pause()
      else close()
    } else if (key === '+' || key === '=') zoomAt(1.25)
    else if (key === '-') zoomAt(0.8)
    else if (key === '0') { fit = true; tx = ty = 0 }
    else if (key === '1') { scale = 1; fit = false; tx = ty = 0; hiRes = true }
    else if (key.toLowerCase() === 'r') rot = (rot + (ev.shiftKey ? 270 : 90)) % 360
    else if (key.toLowerCase() === 'i') showInfo = !showInfo
    else if (key.toLowerCase() === 's') slideshow = !slideshow
    else if (key.toLowerCase() === 'f' || key === 'F11') fullscreen()
    else if (key === 'Delete') del()
    else if (key === 'Enter') api.open(e.path)
    else return
    ev.preventDefault()
    ev.stopPropagation()
  }

  async function fullscreen() {
    const w = getCurrentWindow()
    await w.setFullscreen(!(await w.isFullscreen()))
  }

  function poke() {
    chrome = true
    clearTimeout(hideTimer)
    hideTimer = setTimeout(() => (chrome = false), 2600)
  }

  // Pasek miniatur: renderujemy tylko okno wokół bieżącego zdjęcia.
  const stripFrom = $derived(Math.max(0, v.index - 30))
  const strip = $derived(v.list.slice(stripFrom, v.index + 31))
  let stripEl = $state<HTMLDivElement>()
  $effect(() => {
    v.index
    stripEl?.querySelector('.cur')?.scrollIntoView({ inline: 'center', block: 'nearest', behavior: 'smooth' })
  })
</script>

<svelte:window onkeydown={onKey} />

{#if e}
  <div class="viewer fade-in" class:nochrome={!chrome && !showInfo} onpointermove={poke} role="dialog" tabindex="-1" aria-label="Podgląd">
    <div class="top">
      <button class="icon-btn" title="Zamknij (Esc)" onclick={close}><Icon name="back" /></button>
      <div class="grow title">
        <b class="ellipsis">{e.name}</b>
        <span class="faint">{v.index + 1} / {v.list.length} · {size(e.size)} · {date(e.modified)}{natural.w ? ` · ${natural.w}×${natural.h}` : ''}</span>
      </div>
      {#if k === 'image' || k === 'raw'}
        <button class="icon-btn" title="Pomniejsz (−)" onclick={() => zoomAt(0.8)}><Icon name="zoomOut" /></button>
        <span class="zoomval">{Math.round(effScale * 100)}%</span>
        <button class="icon-btn" title="Powiększ (+)" onclick={() => zoomAt(1.25)}><Icon name="zoomIn" /></button>
        <button class="icon-btn" title="Obróć (R)" onclick={() => (rot = (rot + 90) % 360)}><Icon name="rotate" /></button>
      {/if}
      <button class="icon-btn" class:on={slideshow} title="Pokaz slajdów (S)" onclick={() => (slideshow = !slideshow)}><Icon name={slideshow ? 'pause' : 'play'} /></button>
      <button class="icon-btn" title="Otwórz w programie (Enter)" onclick={() => api.open(e.path)}><Icon name="open" /></button>
      <button class="icon-btn" title="Do Kosza (Del)" onclick={del}><Icon name="trash" /></button>
      <button class="icon-btn" class:on={showInfo} title="Informacje (I)" onclick={() => (showInfo = !showInfo)}><Icon name="info" /></button>
      <button class="icon-btn" title="Pełny ekran (F)" onclick={fullscreen}><Icon name="fullscreen" /></button>
    </div>

    <div class="body">
      <div
        class="stage"
        bind:this={stageEl}
        onwheel={onWheel}
        onpointerdown={down}
        onpointermove={move}
        onpointerup={() => (drag = null)}
        ondblclick={(ev) => (k === 'image' || k === 'raw') && toggleFit(ev)}
        role="presentation"
        class:grab={!fit}
      >
        {#if k === 'image' || k === 'raw'}
          {#key src}
            <img
              src={hiRes && k === 'raw' ? src : src}
              alt={e.name}
              draggable="false"
              class:loaded={loadedSrc === src}
              style="transform: translate({tx}px, {ty}px) scale({effScale}) rotate({rot}deg)"
              style:image-rendering={effScale > 2 ? 'pixelated' : 'auto'}
              onload={(ev) => {
                const im = ev.currentTarget as HTMLImageElement
                natural = { w: im.naturalWidth, h: im.naturalHeight }
                loadedSrc = src
              }}
              onerror={(ev) => ((ev.currentTarget as HTMLImageElement).src = thumbUrl(e.path, 256, 'i'))}
            />
          {/key}
        {:else if k === 'video'}
          {#key e.path}
            {#if WEB_VIDEO.has(e.ext)}
              <!-- svelte-ignore a11y_media_has_caption -->
              <video bind:this={videoEl} src={fileUrl(e.path)} controls autoplay onended={() => slideshow && go(1, true)}
                onerror={() => toast('Tego formatu wideo nie da się odtworzyć w aplikacji – Enter otworzy go w systemowym odtwarzaczu')}></video>
            {:else}
              <div class="nopreview"><div class="big"><Thumb entry={e} px={512} /></div><p class="muted">Format {e.ext.toUpperCase()} – otwórz w odtwarzaczu systemowym</p><button class="btn primary" onclick={() => api.open(e.path)}>Odtwórz</button></div>
            {/if}
          {/key}
        {:else if k === 'audio' && WEB_AUDIO.has(e.ext)}
          {#key e.path}
            <div class="nopreview"><div class="big"><Thumb entry={e} px={512} /></div><audio src={fileUrl(e.path)} controls autoplay></audio></div>
          {/key}
        {:else if k === 'pdf'}
          {#key e.path}<iframe title={e.name} src={fileUrl(e.path)}></iframe>{/key}
        {:else if hasDocPreview(e)}
          <div class="docv">{#key e.path}<DocPreview entry={e} />{/key}</div>
        {:else if text}
          <pre>{text.text}</pre>
        {:else}
          <div class="nopreview"><div class="big"><Thumb entry={e} px={512} /></div><p class="muted">{e.name}</p><button class="btn primary" onclick={() => api.open(e.path)}>Otwórz</button></div>
        {/if}

        {#if v.index > 0}<button class="nav l" title="Poprzedni (←)" onclick={() => go(-1)}><Icon name="back" size={22} /></button>{/if}
        {#if v.index < v.list.length - 1}<button class="nav r" title="Następny (→)" onclick={() => go(1)}><Icon name="fwd" size={22} /></button>{/if}
      </div>

      {#if showInfo}
        <aside class="side fade-in">
          <h3>Informacje</h3>
          <dl>
            <dt>Nazwa</dt><dd>{e.name}</dd>
            <dt>Rozmiar</dt><dd>{size(e.size)}</dd>
            {#if natural.w && (k === 'image')}<dt>Wymiary</dt><dd>{natural.w} × {natural.h}</dd>{/if}
            <dt>Zmodyfikowano</dt><dd>{date(e.modified)}</dd>
            {#each props as p (p.name)}<dt>{p.name}</dt><dd>{p.value}</dd>{/each}
          </dl>
        </aside>
      {/if}
    </div>

    {#if v.list.length > 1}
      <div class="strip" bind:this={stripEl} onwheel={stripWheel}>
        {#each strip as s, j (s.path)}
          <button class="st" class:cur={stripFrom + j === v.index} onclick={() => (app.viewer = { list: v.list, index: stripFrom + j })}>
            <Thumb entry={s} px={64} fit="cover" />
          </button>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .docv {
    position: absolute;
    inset: 0 64px;
    background: var(--bg);
    color: var(--text);
  }
  .viewer {
    position: fixed;
    inset: 0;
    z-index: 200;
    background: #000;
    display: flex;
    flex-direction: column;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px 12px;
    transition: opacity 0.3s;
    z-index: 2;
  }
  .title {
    display: flex;
    flex-direction: column;
    padding: 0 10px;
    min-width: 0;
  }
  .title span {
    font-size: 12px;
  }
  .zoomval {
    font-size: 12px;
    color: var(--dim);
    width: 44px;
    text-align: center;
  }
  .nochrome .top,
  .nochrome .strip,
  .nochrome .nav {
    opacity: 0;
  }
  .nochrome {
    cursor: none;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .stage {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .stage.grab {
    cursor: grab;
  }
  .stage img {
    max-width: none;
    max-height: none;
    transform-origin: center;
    transition: opacity 0.15s;
    opacity: 0;
    position: absolute;
  }
  .stage img.loaded {
    opacity: 1;
  }
  video {
    max-width: 100%;
    max-height: 100%;
    outline: none;
  }
  iframe {
    width: 100%;
    height: 100%;
    border: 0;
    background: #fff;
  }
  pre {
    width: min(1000px, 100%);
    height: 100%;
    margin: 0;
    padding: 20px 24px;
    overflow: auto;
    font: 13px/1.6 var(--mono);
    color: var(--dim);
    white-space: pre-wrap;
    user-select: text;
  }
  .nopreview {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
  }
  .big {
    width: 300px;
    height: 300px;
  }
  audio {
    width: 360px;
  }
  .nav {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.08);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: opacity 0.3s, background 0.12s;
  }
  .nav:hover {
    background: rgba(255, 255, 255, 0.18);
  }
  .nav.l {
    left: 16px;
  }
  .nav.r {
    right: 16px;
  }
  .side {
    width: 320px;
    flex: none;
    background: var(--panel);
    padding: 16px 18px;
    overflow-y: auto;
    border-radius: var(--r-l) 0 0 0;
  }
  .side h3 {
    margin: 0 0 12px;
    font-size: 14px;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 7px 12px;
    font-size: 12px;
    margin: 0;
  }
  dt {
    color: var(--faint);
  }
  dd {
    margin: 0;
    color: var(--dim);
    word-break: break-word;
    user-select: text;
  }
  .strip {
    display: flex;
    gap: 4px;
    padding: 8px 12px 10px;
    overflow-x: auto;
    justify-content: safe center;
    transition: opacity 0.3s;
    scrollbar-width: none;
    flex: none;
  }
  .st {
    width: 56px;
    height: 56px;
    flex: none;
    border-radius: var(--r-s);
    overflow: hidden;
    opacity: 0.45;
    transition: opacity 0.15s, transform 0.15s;
    background: var(--panel);
  }
  .st:hover {
    opacity: 0.8;
  }
  .st.cur {
    opacity: 1;
    transform: scale(1.08);
    outline: 2px solid #fff;
    outline-offset: -2px;
  }
</style>
