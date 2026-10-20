<script lang="ts">
  import Icon from './Icon.svelte'
  import Thumb from './Thumb.svelte'
  import { app, settings, openEntry, openViewer, toast, type Tab } from '../lib/state.svelte'
  import { api, fileUrl, thumbUrl, type Entry, type Prop, type TextPreview, type ZipItem, type DirSize } from '../lib/api'
  import { kindOf, size, date, typeLabel, WEB_IMAGE, WEB_VIDEO, WEB_AUDIO, OFFICE_TEXT, items as itemsLabel, debounce, volumeOf } from '../lib/util'

  let { tab }: { tab: Tab } = $props()

  const sel = $derived(tab.selectedEntries)
  const one = $derived(sel.length === 1 ? sel[0] : null)
  // Krótkie opóźnienie: przytrzymana strzałka nie ładuje każdego pliku po drodze.
  let shown = $state<Entry | null>(null)
  const setShown = debounce((e: Entry | null) => (shown = e), 90)
  $effect(() => setShown(one))

  const k = $derived(shown ? kindOf(shown) : 'other')
  let text = $state<TextPreview | null>(null)
  let zip = $state<ZipItem[] | null>(null)
  let meta = $state<Prop[]>([])
  let dsize = $state<DirSize | null>(null)
  let dsizeBusy = $state(false)
  let moreProps = $state(false)
  let token = 0

  $effect(() => {
    const e = shown
    const t = ++token
    text = null
    zip = null
    meta = []
    dsize = null
    dsizeBusy = false
    if (!e) return
    if (!e.cloud) {
      if (!e.dir && (k === 'text' || k === 'code' || (k === 'other' && e.size < 2_000_000)))
        api.readText(e.path, 200_000).then((r) => t === token && (text = r)).catch(() => {})
      else if (OFFICE_TEXT.has(e.ext)) api.officeText(e.path).then((r) => t === token && (text = r)).catch(() => {})
      else if (e.ext === 'zip' || e.ext === 'cbz' || e.ext === 'apk' || e.ext === 'jar') api.zipList(e.path).then((r) => t === token && (zip = r)).catch(() => {})
    }
    api.shellProps(e.path).then((r) => t === token && (meta = r)).catch(() => {})
    if (e.dir && !e.path.startsWith('\\\\')) {
      dsizeBusy = true
      api.dirSize(e.path).then((r) => { if (t === token) { dsize = r; dsizeBusy = false } })
    }
  })

  const SKIP = new Set(['Nazwa', 'Name', 'Nazwa pliku', 'Rozmiar', 'Size', 'Typ elementu', 'Item type', 'Folder', 'Ścieżka folderu', 'Folder path', 'Ścieżka', 'Path', 'Data modyfikacji', 'Date modified', 'Data utworzenia', 'Date created', 'Atrybuty', 'Attributes', 'Data dostępu', 'Date accessed', 'Właściciel', 'Owner', 'Komputer', 'Computer', 'Nazwa folderu', 'Folder name', 'Typ pliku', 'File extension', 'Rozszerzenie pliku', 'Typ zawartości', 'Content type', 'Rodzaj', 'Kind', 'Postęp pobierania', 'Stan udostępniania', 'Sharing status', 'Klasyfikacja', 'Perceived type', 'Postrzegany typ'])
  const shownProps = $derived(meta.filter((p) => !SKIP.has(p.name)))

  const curDrive = $derived(app.drives.find((d) => !tab.special && volumeOf(d.path) === volumeOf(tab.path)))
  const selSize = $derived(sel.reduce((a, e) => a + (e.dir ? 0 : e.size), 0))

  let resizing = false
  function onResize(e: PointerEvent) {
    if (resizing) settings.previewWidth = Math.max(260, Math.min(760, window.innerWidth - e.clientX))
  }
</script>

<aside class="preview" style:width="{settings.previewWidth}px">
  <div class="handle" role="separator" onpointerdown={(e) => { resizing = true; (e.target as HTMLElement).setPointerCapture(e.pointerId) }} onpointermove={onResize} onpointerup={() => (resizing = false)}></div>

  {#if shown && one}
    {@const e = shown}
    <div class="stage" class:tall={k === 'pdf' || !!text || !!zip}>
      {#if e.cloud}
        <div class="thumbbox"><Thumb entry={e} px={300} /></div>
        <p class="faint center">Plik tylko w chmurze – podgląd pobrałby go z OneDrive.</p>
      {:else if k === 'image' && WEB_IMAGE.has(e.ext)}
        <button class="media" ondblclick={() => openViewer(tab, e)} title="Dwuklik: pełny ekran">
          <img src={fileUrl(e.path)} alt="" draggable="false" />
        </button>
      {:else if k === 'image' || k === 'raw'}
        <button class="media" ondblclick={() => openViewer(tab, e)}>
          <img src={thumbUrl(e.path, settings.previewWidth, 't')} alt="" draggable="false" onerror={(ev) => ((ev.currentTarget as HTMLImageElement).src = thumbUrl(e.path, 256, 'i'))} />
        </button>
      {:else if k === 'video' && WEB_VIDEO.has(e.ext)}
        {#key e.path}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video src={fileUrl(e.path)} controls muted={settings.playVideoPreview} autoplay={settings.playVideoPreview} loop preload="metadata" poster={thumbUrl(e.path, settings.previewWidth, 't')}></video>
        {/key}
      {:else if k === 'audio' && WEB_AUDIO.has(e.ext)}
        <div class="thumbbox"><Thumb entry={e} px={300} /></div>
        {#key e.path}<audio src={fileUrl(e.path)} controls preload="metadata"></audio>{/key}
      {:else if k === 'pdf'}
        {#key e.path}<iframe title="PDF" src={fileUrl(e.path) + '#toolbar=0&navpanes=0&view=FitH'}></iframe>{/key}
      {:else if text && text.kind !== 'binary' && text.text}
        {#if OFFICE_TEXT.has(e.ext)}<div class="thumbbox small"><Thumb entry={e} px={200} /></div>{/if}
        <pre class:doc={text.kind === 'document'}>{text.text}{text.truncated ? '\n\n…' : ''}</pre>
      {:else if zip}
        <div class="zip">
          <p class="faint">{itemsLabel(zip.length)} · {size(zip.reduce((a, z) => a + z.size, 0))} po rozpakowaniu</p>
          {#each zip.slice(0, 300) as z (z.name)}
            <div class="zrow"><Icon name={z.dir ? 'folder' : 'file'} size={14} /><span class="ellipsis grow">{z.name}</span><span class="faint">{z.dir ? '' : size(z.size)}</span></div>
          {/each}
        </div>
      {:else}
        <div class="thumbbox"><Thumb entry={e} px={Math.min(settings.previewWidth, 400)} iconOnly={e.dir} /></div>
      {/if}
    </div>

    <div class="info">
      <h3>{e.name}</h3>
      <p class="faint">{typeLabel(e)}{e.dir ? '' : ` · ${size(e.size)}`}</p>
      <div class="actions">
        <button class="act main" onclick={() => openEntry(tab, e)}><Icon name="open" size={17} /><span>Otwórz</span></button>
        {#if !e.dir}<button class="act" onclick={() => api.openWith(e.path)}><Icon name="app" size={17} /><span>Otwórz w…</span></button>{/if}
        <button class="act" onclick={() => (app.dialog = { type: 'props', entries: [e] })}><Icon name="info" size={17} /><span>Właściwości</span></button>
      </div>
      <dl>
        {#if e.dir}
          <dt>Zawartość</dt>
          <dd>{#if dsize}{itemsLabel(dsize.files)} w {dsize.dirs} folderach · {size(dsize.size)}{:else if dsizeBusy}<span class="faint">liczę…</span>{:else}—{/if}</dd>
        {/if}
        <dt>Zmodyfikowano</dt><dd>{date(e.modified)}</dd>
        <dt>Utworzono</dt><dd>{date(e.created)}</dd>
        {#each moreProps ? shownProps : shownProps.slice(0, 8) as p (p.name)}
          <dt>{p.name}</dt><dd>{p.value}</dd>
        {/each}
      </dl>
      {#if shownProps.length > 8}
        <button class="more" onclick={() => (moreProps = !moreProps)}>{moreProps ? 'Mniej' : `Więcej szczegółów (${shownProps.length - 8})`}</button>
      {/if}
      <button class="path" title="Kliknij, aby skopiować ścieżkę" onclick={() => { navigator.clipboard.writeText(e.path); toast('Skopiowano ścieżkę', 'ok') }}>
        <span class="grow">{e.path}</span><Icon name="copy" size={14} />
      </button>
    </div>
  {:else if sel.length > 1}
    <div class="multi">
      <div class="stack">
        {#each sel.slice(0, 3) as e, i (e.path)}
          <div class="st" style="transform: rotate({(i - 1) * 6}deg) translateY({i * -4}px)"><Thumb entry={e} px={160} fit="cover" iconOnly={e.dir} /></div>
        {/each}
      </div>
      <h3>Zaznaczono {itemsLabel(sel.length)}</h3>
      <p class="faint">
        {sel.filter((e) => e.dir).length} folderów · {sel.filter((e) => !e.dir).length} plików · {size(selSize)}
      </p>
      <button class="btn sm" onclick={() => (app.dialog = { type: 'props', entries: sel })}><Icon name="info" size={15} />Właściwości</button>
    </div>
  {:else}
    <div class="multi">
      <Icon name={tab.special ? 'pc' : 'folder'} size={40} />
      <h3>{tab.title}</h3>
      {#if !tab.special}
        <p class="faint">{itemsLabel(tab.items.length)}</p>
        {#if curDrive?.total}
          <div class="drive">
            <span class="bar"><i style:width="{(1 - curDrive.free / curDrive.total) * 100}%"></i></span>
            <p class="faint">{size(curDrive.free)} wolne z {size(curDrive.total)}</p>
          </div>
        {/if}
      {/if}
      <p class="ghost">Zaznacz plik, aby zobaczyć podgląd</p>
    </div>
  {/if}
</aside>

<style>
  .preview {
    position: relative;
    flex: none;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border-radius: var(--r-l) 0 0 0;
    min-height: 0;
    overflow-y: auto;
  }
  .handle {
    position: absolute;
    left: -3px;
    top: 0;
    width: 6px;
    height: 100%;
    cursor: col-resize;
    z-index: 3;
  }
  .stage {
    padding: 14px 14px 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    flex: none;
  }
  .stage.tall {
    flex: 1;
    min-height: 320px;
  }
  .media {
    width: 100%;
    display: flex;
    justify-content: center;
    border-radius: var(--r);
    overflow: hidden;
    background: #000;
  }
  .media img {
    max-width: 100%;
    max-height: 46vh;
    object-fit: contain;
    display: block;
  }
  video {
    width: 100%;
    max-height: 46vh;
    border-radius: var(--r);
    background: #000;
  }
  audio {
    width: 100%;
  }
  iframe {
    width: 100%;
    flex: 1;
    min-height: 300px;
    border: 0;
    border-radius: var(--r);
    background: #fff;
  }
  .thumbbox {
    width: 100%;
    aspect-ratio: 1 / 0.8;
    max-height: 40vh;
    border-radius: var(--r);
    overflow: hidden;
  }
  .thumbbox.small {
    aspect-ratio: auto;
    height: 160px;
  }
  pre {
    width: 100%;
    flex: 1;
    margin: 0;
    padding: 12px 14px;
    background: #000;
    border-radius: var(--r);
    font: 12px/1.55 var(--mono);
    color: var(--dim);
    white-space: pre-wrap;
    word-break: break-word;
    overflow: auto;
    user-select: text;
    max-height: 60vh;
  }
  pre.doc {
    font: 13px/1.6 var(--font);
  }
  .zip {
    width: 100%;
    max-height: 50vh;
    overflow: auto;
    font-size: 12px;
  }
  .zrow {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 4px;
    color: var(--dim);
  }
  .info {
    padding: 16px 18px 24px;
  }
  h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    word-break: break-word;
    user-select: text;
  }
  .info > p {
    margin: 4px 0 0;
  }
  .actions {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 1fr;
    gap: 6px;
    margin: 16px 0 6px;
  }
  .act {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 58px;
    border-radius: 14px;
    background: var(--high);
    color: var(--dim);
    font-size: 12px;
    transition: background 0.12s, color 0.12s, transform 0.12s;
  }
  .act:hover {
    background: var(--higher);
    color: var(--text);
  }
  .act:active {
    transform: scale(0.96);
  }
  .act.main {
    background: #fff;
    color: #000;
    font-weight: 600;
  }
  .act.main:hover {
    background: #e8e8e8;
  }
  .btn.sm {
    height: 30px;
    font-size: 12px;
    padding: 0 12px;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 7px 14px;
    margin: 14px 0 0;
    font-size: 12px;
  }
  dt {
    color: var(--faint);
    white-space: nowrap;
  }
  dd {
    margin: 0;
    text-align: right;
    color: var(--dim);
    word-break: break-word;
    user-select: text;
  }
  .more {
    display: block;
    margin: 10px 0 0 auto;
    font-size: 12px;
    color: var(--faint);
  }
  .more:hover {
    color: var(--text);
  }
  .path {
    width: 100%;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    margin-top: 16px;
    padding: 10px 12px;
    border-radius: 12px;
    background: var(--high);
    color: var(--faint);
    font-size: 11px;
    text-align: left;
    word-break: break-all;
    transition: background 0.12s, color 0.12s;
  }
  .path:hover {
    background: var(--higher);
    color: var(--text);
  }
  .multi {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 30px;
    text-align: center;
    color: var(--dim);
  }
  .multi p {
    margin: 0;
  }
  .ghost {
    color: var(--ghost);
    font-size: 12px;
    margin-top: 10px !important;
  }
  .stack {
    position: relative;
    width: 150px;
    height: 150px;
    margin-bottom: 16px;
  }
  .st {
    position: absolute;
    inset: 0;
    border-radius: 14px;
    overflow: hidden;
    background: var(--high);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.6);
  }
  .drive {
    width: 200px;
    margin-top: 8px;
  }
  .drive .bar {
    display: block;
    margin-bottom: 6px;
  }
  .center {
    text-align: center;
    font-size: 12px;
  }
</style>
