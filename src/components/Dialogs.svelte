<script lang="ts">
  import Icon from './Icon.svelte'
  import Thumb from './Thumb.svelte'
  import { app, activeTab, navigate, refreshDrives, refreshSync, toast } from '../lib/state.svelte'
  import { api, type ConflictPolicy, type Entry, type Prop, type SyncProfile, type SyncMode, type SyncCounts } from '../lib/api'
  import { size, date, dateFull, typeLabel, items as itemsLabel, basename } from '../lib/util'

  const d = $derived(app.dialog)

  function close() {
    const x = app.dialog
    app.dialog = null
    if (!x) return
    if (x.type === 'prompt') x.resolve(null)
    if (x.type === 'confirm') x.resolve(false)
    if (x.type === 'conflict') x.resolve(null)
  }

  function onKey(ev: KeyboardEvent) {
    if (!app.dialog) return
    if (ev.key === 'Escape') {
      ev.preventDefault()
      ev.stopPropagation()
      close()
    }
  }

  // ---------- prompt ----------
  let promptValue = $state('')
  $effect(() => {
    if (d?.type === 'prompt') promptValue = d.value
  })
  function promptMount(node: HTMLInputElement) {
    node.focus()
    if (d?.type === 'prompt' && d.selectStem) {
      const dot = node.value.lastIndexOf('.')
      node.setSelectionRange(0, dot > 0 ? dot : node.value.length)
    } else node.select()
  }
  function promptOk() {
    if (d?.type !== 'prompt') return
    const r = d.resolve
    app.dialog = null
    r(promptValue.trim() || null)
  }

  // ---------- konflikty ----------
  function conflictPick(p: ConflictPolicy) {
    if (d?.type !== 'conflict') return
    const r = d.resolve
    app.dialog = null
    r(p)
  }

  // ---------- właściwości ----------
  let props = $state<Prop[]>([])
  let totals = $state<{ size: number; files: number; dirs: number } | null>(null)
  let hash = $state('')
  let hashing = $state(false)
  $effect(() => {
    if (d?.type !== 'props') return
    const entries = d.entries
    props = []
    totals = null
    hash = ''
    hashing = false
    if (entries.length === 1) api.shellProps(entries[0].path).then((p) => (props = p))
    Promise.all(entries.map((e) => (e.dir ? api.dirSize(e.path) : Promise.resolve({ size: e.size, files: 1, dirs: 0 })))).then((rs) => {
      totals = rs.reduce((a, r) => ({ size: a.size + r.size, files: a.files + r.files, dirs: a.dirs + r.dirs }), { size: 0, files: 0, dirs: entries.filter((e) => e.dir).length })
    })
  })
  async function doHash(e: Entry) {
    hashing = true
    try {
      hash = await api.hash(e.path)
    } catch (err) {
      toast(String(err), 'error')
    }
    hashing = false
  }

  // ---------- dysk sieciowy ----------
  let unc = $state('')
  let letter = $state('')
  let persistent = $state(true)
  let busy = $state(false)
  const freeLetters = $derived('ZYXWVUTSRQPONMLKJIHGFED'.split('').filter((l) => !app.drives.some((x) => x.letter[0].toUpperCase() === l)))
  $effect(() => {
    if (d?.type === 'network') {
      unc = ''
      letter = freeLetters[0] ?? ''
      busy = false
    }
  })
  function normUnc(s: string) {
    s = s.trim().replace(/^smb:\/\//i, '').replace(/^file:\/\//i, '').replace(/\//g, '\\')
    if (!s.startsWith('\\\\')) s = '\\\\' + s.replace(/^\\+/, '')
    return s.replace(/\\+$/, '')
  }
  async function mapIt() {
    const remote = normUnc(unc)
    if (remote.split('\\').filter(Boolean).length < 2) return toast('Podaj adres w postaci \\\\serwer\\udział', 'error')
    busy = true
    try {
      const path = await api.mapDrive(letter || null, remote, persistent)
      app.dialog = null
      toast(`Podłączono ${letter ? letter + ':' : remote}`, 'ok')
      await refreshDrives()
      navigate(activeTab(), path)
    } catch (err) {
      toast(String(err), 'error')
    }
    busy = false
  }
  function openUnc() {
    const remote = normUnc(unc)
    app.dialog = null
    navigate(activeTab(), remote)
  }

  // ---------- edytor synchronizacji ----------
  let sp = $state<SyncProfile>(blank())
  let excludeText = $state('')
  function blank(source = ''): SyncProfile {
    return {
      id: '', name: source ? basename(source) : '', source, target: { path: '', rel: '', serial: 0, label: '', network: false },
      mode: 'mirror', enabled: true, continuous: true, interval_min: 15, versioning: true, exclude: [], include_hidden: false,
    }
  }
  $effect(() => {
    if (d?.type === 'sync') {
      sp = d.profile ? structuredClone($state.snapshot(d.profile)) as SyncProfile : blank(d.source ?? '')
      excludeText = sp.exclude.join('\n')
      busy = false
    }
  })
  async function pick(which: 'source' | 'target') {
    const p = await api.pickFolder(which === 'source' ? sp.source || undefined : sp.target.path || undefined)
    if (!p) return
    if (which === 'source') {
      sp.source = p
      if (!sp.name) sp.name = basename(p)
    } else sp.target = { ...sp.target, path: p }
  }
  async function saveSync() {
    busy = true
    try {
      sp.exclude = excludeText.split('\n').map((s) => s.trim()).filter(Boolean)
      if (!sp.name.trim()) sp.name = basename(sp.source)
      await api.syncSave($state.snapshot(sp) as SyncProfile)
      app.dialog = null
      await refreshSync()
      toast('Zapisano synchronizację – pierwszy przebieg rusza, gdy cel będzie dostępny', 'ok')
    } catch (err) {
      toast(String(err), 'error')
    }
    busy = false
  }
  const MODES: { id: SyncMode; title: string; text: string }[] = [
    { id: 'mirror', title: 'Kopia lustrzana', text: 'Cel staje się dokładną kopią źródła. Usunięte pliki znikają też z celu (z wersjonowaniem – trafiają do .ofp-versions).' },
    { id: 'twoway', title: 'Dwukierunkowa', text: 'Zmiany po obu stronach są łączone. Gdy ten sam plik zmieniono w dwóch miejscach, obie wersje zostają zachowane.' },
    { id: 'backup', title: 'Kopia zapasowa', text: 'Nowe i zmienione pliki trafiają do celu, ale nic nie jest z niego usuwane.' },
  ]

  // ---------- podgląd planu ----------
  let plan = $state<{ counts: SyncCounts; items: string[] } | null>(null)
  let planErr = $state('')
  $effect(() => {
    if (d?.type === 'syncPreview') {
      plan = null
      planErr = ''
      api.syncPreview(d.id).then((p) => (plan = p)).catch((e) => (planErr = String(e)))
    }
  })
</script>

<svelte:window onkeydowncapture={onKey} />

{#if d}
  <div class="backdrop fade-in" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
    {#if d.type === 'prompt'}
      <form class="dlg small" onsubmit={(e) => { e.preventDefault(); promptOk() }}>
        <h2>{d.title}</h2>
        {#if d.label}<p class="muted">{d.label}</p>{/if}
        <input bind:value={promptValue} use:promptMount spellcheck="false" />
        <div class="btns"><button type="button" class="btn ghost" onclick={close}>Anuluj</button><button class="btn primary" type="submit">{d.ok}</button></div>
      </form>
    {:else if d.type === 'confirm'}
      <div class="dlg small">
        <h2>{d.title}</h2>
        <p class="muted pre">{d.text}</p>
        <div class="btns">
          <button class="btn ghost" onclick={close}>Anuluj</button>
          <!-- svelte-ignore a11y_autofocus -->
          <button class="btn {d.danger ? 'danger' : 'primary'}" autofocus onclick={() => { const r = d.resolve; app.dialog = null; r(true) }}>{d.ok}</button>
        </div>
      </div>
    {:else if d.type === 'conflict'}
      <div class="dlg">
        <h2>{d.conflicts.length === 1 ? 'Element już istnieje' : `${d.conflicts.length} elementów już istnieje`}</h2>
        <p class="muted">W folderze docelowym są już elementy o tych samych nazwach.</p>
        <div class="conf-list">
          {#each d.conflicts.slice(0, 6) as c (c.src)}
            <div class="conf">
              <b class="ellipsis">{c.name}</b>
              <div class="cmp">
                <span>Nowy: {c.dir ? 'folder' : size(c.src_size)} · {date(c.src_modified)}{c.src_modified > c.dst_modified + 2000 ? ' · nowszy' : ''}</span>
                <span>Istniejący: {c.dir ? 'folder' : size(c.dst_size)} · {date(c.dst_modified)}{c.dst_modified > c.src_modified + 2000 ? ' · nowszy' : ''}</span>
              </div>
            </div>
          {/each}
          {#if d.conflicts.length > 6}<p class="faint">…i {d.conflicts.length - 6} więcej</p>{/if}
        </div>
        <div class="choices">
          <button onclick={() => conflictPick('replace')}><Icon name="copy" /><span><b>Zastąp</b><small>{d.conflicts.some((c) => c.dir) ? 'Foldery zostaną scalone' : 'Istniejące pliki zostaną nadpisane'}</small></span></button>
          <button onclick={() => conflictPick('newer')}><Icon name="history" /><span><b>Zastąp tylko starsze</b><small>Nadpisz, jeśli kopiowany plik jest nowszy</small></span></button>
          <button onclick={() => conflictPick('rename')}><Icon name="filePlus" /><span><b>Zachowaj oba</b><small>Nowe dostaną nazwę „(2)”</small></span></button>
          <button onclick={() => conflictPick('skip')}><Icon name="fwd" /><span><b>Pomiń</b><small>Nie ruszaj istniejących</small></span></button>
        </div>
        <div class="btns"><button class="btn ghost" onclick={close}>Anuluj {d.mode === 'move' ? 'przenoszenie' : 'kopiowanie'}</button></div>
      </div>
    {:else if d.type === 'props'}
      {@const one = d.entries.length === 1 ? d.entries[0] : null}
      <div class="dlg wide">
        <div class="ph">
          <div class="pt">{#if one}<Thumb entry={one} px={96} iconOnly={one.dir} />{:else}<Icon name="copy" size={40} />{/if}</div>
          <div class="grow">
            <h2 class="sel">{one ? one.name : `${itemsLabel(d.entries.length)}`}</h2>
            <p class="muted">{one ? typeLabel(one) : 'Wiele elementów'}</p>
          </div>
          <button class="icon-btn" onclick={close}><Icon name="close" /></button>
        </div>
        <dl>
          <dt>Rozmiar</dt>
          <dd>{#if totals}{size(totals.size)} <span class="faint">({totals.size.toLocaleString('pl-PL')} B)</span>{:else}<span class="faint">liczę…</span>{/if}</dd>
          {#if totals && (totals.dirs || d.entries.length > 1)}<dt>Zawiera</dt><dd>{itemsLabel(totals.files)} plików, {totals.dirs} folderów</dd>{/if}
          {#if one}
            <dt>Lokalizacja</dt><dd class="sel">{one.path}</dd>
            <dt>Utworzono</dt><dd>{dateFull(one.created)}</dd>
            <dt>Zmodyfikowano</dt><dd>{dateFull(one.modified)}</dd>
            <dt>Atrybuty</dt>
            <dd>{[one.readonly && 'tylko do odczytu', one.hidden && 'ukryty', one.system && 'systemowy', one.link && 'dowiązanie', one.cloud && 'tylko w chmurze'].filter(Boolean).join(', ') || 'brak'}</dd>
            {#each props as p (p.name)}<dt>{p.name}</dt><dd class="sel">{p.value}</dd>{/each}
            {#if !one.dir}
              <dt>SHA-256</dt>
              <dd>{#if hash}<code class="sel">{hash}</code> <button class="link" onclick={() => { navigator.clipboard.writeText(hash); toast('Skopiowano skrót') }}>kopiuj</button>{:else}<button class="link" disabled={hashing} onclick={() => doHash(one)}>{hashing ? 'liczę…' : 'Oblicz'}</button>{/if}</dd>
            {/if}
          {/if}
        </dl>
        <div class="btns">
          {#if one}<button class="btn" onclick={() => api.systemProps(one.path)}><Icon name="settings" size={16} />Właściwości systemowe (uprawnienia, wersje)</button>{/if}
          <button class="btn primary" onclick={close}>Zamknij</button>
        </div>
      </div>
    {:else if d.type === 'network'}
      <form class="dlg" onsubmit={(e) => { e.preventDefault(); mapIt() }}>
        <h2>Podłącz dysk sieciowy</h2>
        <p class="muted">Udział SMB z NAS-a, routera lub innego komputera. Jeśli potrzebne jest hasło, Windows zapyta o nie w swoim oknie i może je zapamiętać.</p>
        <label class="field"><span>Adres</span>
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:value={unc} placeholder="\\nas\zdjecia  lub  192.168.1.10\dane" spellcheck="false" autofocus />
        </label>
        <div class="row2">
          <label class="field"><span>Litera dysku</span>
            <select bind:value={letter}>
              <option value="">(bez litery)</option>
              {#each freeLetters as l}<option value={l}>{l}:</option>{/each}
            </select>
          </label>
          <label class="check"><button type="button" class="switch" class:on={persistent} onclick={() => (persistent = !persistent)} aria-label="Przywracaj przy logowaniu"></button>Przywracaj przy logowaniu</label>
        </div>
        <div class="btns">
          <button type="button" class="btn ghost" onclick={close}>Anuluj</button>
          <button type="button" class="btn" disabled={!unc.trim()} onclick={openUnc}>Otwórz bez mapowania</button>
          <button class="btn primary" type="submit" disabled={busy || !unc.trim()}>{busy ? 'Łączę…' : 'Podłącz'}</button>
        </div>
      </form>
    {:else if d.type === 'sync'}
      <form class="dlg wide" onsubmit={(e) => { e.preventDefault(); saveSync() }}>
        <h2>{d.profile ? 'Edytuj synchronizację' : 'Nowa synchronizacja'}</h2>
        <p class="muted">Gdy dysk docelowy jest podłączony, zmiany są przenoszone na bieżąco. Gdy go nie ma – aplikacja pamięta, kiedy był, i nadrobi wszystko po podłączeniu (także pod inną literą).</p>
        <label class="field"><span>Nazwa</span><input bind:value={sp.name} placeholder="np. Zdjęcia → dysk USB" /></label>
        <div class="row2">
          <label class="field grow"><span>Folder źródłowy</span><div class="pick"><input bind:value={sp.source} placeholder="C:\Users\…\Zdjęcia" spellcheck="false" /><button type="button" class="btn" onclick={() => pick('source')}>Wybierz…</button></div></label>
        </div>
        <div class="row2">
          <label class="field grow"><span>Folder docelowy (dysk zewnętrzny lub sieciowy)</span><div class="pick"><input bind:value={sp.target.path} placeholder="E:\Kopie\Zdjęcia  lub  \\nas\kopie" spellcheck="false" /><button type="button" class="btn" onclick={() => pick('target')}>Wybierz…</button></div></label>
        </div>
        <div class="modes">
          {#each MODES as m (m.id)}
            <button type="button" class="mode" class:on={sp.mode === m.id} onclick={() => (sp.mode = m.id)}><b>{m.title}</b><small>{m.text}</small></button>
          {/each}
        </div>
        <div class="opts">
          <label class="check"><button type="button" class="switch" class:on={sp.continuous} onclick={() => (sp.continuous = !sp.continuous)} aria-label="Na bieżąco"></button>Na bieżąco (obserwuj zmiany)</label>
          <label class="check"><button type="button" class="switch" class:on={sp.versioning} onclick={() => (sp.versioning = !sp.versioning)} aria-label="Wersjonowanie"></button>Zachowuj nadpisane i usunięte wersje</label>
          <label class="check"><button type="button" class="switch" class:on={sp.include_hidden} onclick={() => (sp.include_hidden = !sp.include_hidden)} aria-label="Ukryte"></button>Uwzględniaj pliki ukryte</label>
          <label class="check">Pełne porównanie co <input type="number" min="1" max="1440" bind:value={sp.interval_min} class="num" /> min</label>
        </div>
        <label class="field"><span>Pomijaj (jeden wzorzec w linii, np. *.tmp, node_modules, Cache\*)</span><textarea rows="3" bind:value={excludeText} spellcheck="false"></textarea></label>
        <div class="btns">
          <button type="button" class="btn ghost" onclick={close}>Anuluj</button>
          <button class="btn primary" type="submit" disabled={busy || !sp.source || !sp.target.path}>{busy ? 'Zapisuję…' : 'Zapisz i uruchom'}</button>
        </div>
      </form>
    {:else if d.type === 'syncPreview'}
      <div class="dlg wide">
        <h2>Co zrobi synchronizacja</h2>
        {#if planErr}<p class="muted">{planErr}</p>
        {:else if !plan}<p class="muted">Porównuję foldery…</p>
        {:else}
          <div class="counts">
            <span><b>{plan.counts.to_target}</b> do celu</span>
            <span><b>{plan.counts.to_source}</b> do źródła</span>
            <span><b>{plan.counts.del_target + plan.counts.del_source}</b> usunięć</span>
            <span><b>{plan.counts.conflicts}</b> konfliktów</span>
            <span><b>{size(plan.counts.bytes)}</b> do skopiowania</span>
          </div>
          <div class="plan">
            {#each plan.items as it}<div class="ellipsis">{it}</div>{:else}<p class="muted">Wszystko jest aktualne.</p>{/each}
          </div>
        {/if}
        <div class="btns">
          <button class="btn ghost" onclick={close}>Zamknij</button>
          {#if plan?.items.length}<button class="btn primary" onclick={() => { api.syncNow(d.id, true); app.dialog = null }}>Synchronizuj teraz</button>{/if}
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 300;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }
  .dlg {
    width: 560px;
    max-width: 100%;
    max-height: 100%;
    overflow-y: auto;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 20px;
    padding: 22px 24px 18px;
    box-shadow: 0 40px 100px rgba(0, 0, 0, 0.8);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .dlg.small {
    width: 420px;
  }
  .dlg.wide {
    width: 680px;
  }
  h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }
  p {
    margin: 0;
  }
  .pre {
    white-space: pre-line;
  }
  .btns {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 8px;
    flex-wrap: wrap;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
    color: var(--faint);
  }
  .field input,
  .field select,
  .field textarea {
    font-size: 13px;
  }
  textarea {
    resize: vertical;
    font-family: var(--mono);
  }
  .row2 {
    display: flex;
    gap: 14px;
    align-items: flex-end;
  }
  .pick {
    display: flex;
    gap: 8px;
  }
  .pick input {
    flex: 1;
    min-width: 0;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--dim);
    min-height: 36px;
  }
  .num {
    width: 70px;
    padding: 4px 8px;
  }
  .modes {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  .mode {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 14px;
    border-radius: 14px;
    background: var(--high);
    text-align: left;
    outline: 1px solid transparent;
  }
  .mode small {
    color: var(--faint);
    font-size: 11.5px;
    line-height: 1.4;
  }
  .mode.on {
    background: var(--higher);
    outline-color: #fff;
  }
  .opts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 2px 16px;
  }
  .conf-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 220px;
    overflow-y: auto;
  }
  .conf {
    background: var(--high);
    border-radius: 12px;
    padding: 10px 12px;
  }
  .cmp {
    display: flex;
    flex-direction: column;
    font-size: 12px;
    color: var(--faint);
    margin-top: 4px;
  }
  .choices {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .choices button {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    padding: 12px 14px;
    border-radius: 14px;
    background: var(--high);
    text-align: left;
  }
  .choices button:hover {
    background: var(--higher);
    outline: 1px solid var(--line-strong);
  }
  .choices span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .choices small {
    color: var(--faint);
    font-size: 12px;
  }
  .ph {
    display: flex;
    gap: 16px;
    align-items: center;
  }
  .pt {
    width: 72px;
    height: 72px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  dl {
    display: grid;
    grid-template-columns: 150px 1fr;
    gap: 8px 16px;
    margin: 6px 0 0;
    font-size: 12.5px;
    max-height: 52vh;
    overflow-y: auto;
  }
  dt {
    color: var(--faint);
  }
  dd {
    margin: 0;
    color: var(--dim);
    word-break: break-word;
  }
  .sel {
    user-select: text;
  }
  code {
    font: 11px var(--mono);
    word-break: break-all;
  }
  .link {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
    font-size: 12px;
  }
  .counts {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .counts span {
    background: var(--high);
    border-radius: 999px;
    padding: 6px 12px;
    font-size: 12px;
    color: var(--dim);
  }
  .counts b {
    color: var(--text);
  }
  .plan {
    max-height: 46vh;
    overflow-y: auto;
    font: 12px/1.7 var(--mono);
    color: var(--dim);
    background: #000;
    border-radius: 12px;
    padding: 10px 14px;
  }
</style>
