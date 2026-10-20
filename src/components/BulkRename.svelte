<script lang="ts">
  import Icon from './Icon.svelte'
  import { app, activeTab, load, toast, ensureMedia, mediaInfo, media } from '../lib/state.svelte'
  import { api, type Entry } from '../lib/api'
  import { collator, parent } from '../lib/util'

  let { entries }: { entries: Entry[] } = $props()

  let pattern = $state('{nazwa}')
  let find = $state('')
  let replace = $state('')
  let regex = $state(false)
  let matchCase = $state(false)
  let start = $state(1)
  let pad = $state(3)
  let caseMode = $state<'keep' | 'lower' | 'upper' | 'title'>('keep')
  let extMode = $state<'keep' | 'lower' | 'upper'>('keep')
  let order = $state<'list' | 'name' | 'taken' | 'modified'>('list')
  let busy = $state(false)

  // Daty wykonania z EXIF – dociągane w tle, podgląd przelicza się sam.
  $effect(() => {
    ensureMedia(entries)
  })

  const TOKENS: [string, string][] = [
    ['{nazwa}', 'Obecna nazwa'],
    ['{nr}', 'Numer kolejny'],
    ['{data}', 'Data wykonania (EXIF) lub modyfikacji'],
    ['{czas}', 'Godzina'],
    ['{folder}', 'Nazwa folderu'],
    ['{rozmiar}', 'Wymiary zdjęcia'],
  ]
  const PRESETS: [string, string][] = [
    ['Data + numer', '{data}_{nr}'],
    ['Data i godzina', '{data} {czas}'],
    ['Folder + numer', '{folder} {nr}'],
    ['Numer + nazwa', '{nr} {nazwa}'],
  ]

  const two = (n: number) => String(n).padStart(2, '0')
  const stem = (e: Entry) => (e.dir || !e.ext ? e.name : e.name.slice(0, e.name.length - e.ext.length - 1))
  const taken = (e: Entry) => (media.ver, mediaInfo.get(e.path)?.taken || e.modified)

  const sorted = $derived.by(() => {
    media.ver
    const l = [...entries]
    if (order === 'name') l.sort((a, b) => collator.compare(a.name, b.name))
    else if (order === 'taken') l.sort((a, b) => taken(a) - taken(b))
    else if (order === 'modified') l.sort((a, b) => a.modified - b.modified)
    return l
  })

  function titleCase(s: string) {
    return s.toLocaleLowerCase('pl').replace(/(^|[\s_\-.])(\p{L})/gu, (_, a, b) => a + b.toLocaleUpperCase('pl'))
  }

  function build(e: Entry, i: number): string {
    const t = new Date(taken(e))
    const m = mediaInfo.get(e.path)
    let s = pattern
      .replaceAll('{nazwa}', stem(e))
      .replaceAll('{nr}', String(start + i).padStart(pad, '0'))
      .replaceAll('{data}', `${t.getFullYear()}-${two(t.getMonth() + 1)}-${two(t.getDate())}`)
      .replaceAll('{czas}', `${two(t.getHours())}.${two(t.getMinutes())}.${two(t.getSeconds())}`)
      .replaceAll('{folder}', (parent(e.path) ?? '').split('\\').filter(Boolean).pop()?.replace(':', '') ?? '')
      .replaceAll('{rozmiar}', m?.w ? `${m.w}x${m.h}` : '')
    if (find) {
      try {
        const flags = matchCase ? 'g' : 'gi'
        const re = regex ? new RegExp(find, flags) : new RegExp(find.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), flags)
        s = s.replace(re, replace)
      } catch {
        /* niepełne wyrażenie podczas pisania */
      }
    }
    if (caseMode === 'lower') s = s.toLocaleLowerCase('pl')
    else if (caseMode === 'upper') s = s.toLocaleUpperCase('pl')
    else if (caseMode === 'title') s = titleCase(s)
    s = s.trim()
    if (e.dir || !e.ext) return s
    const ext = extMode === 'lower' ? e.ext.toLowerCase() : extMode === 'upper' ? e.ext.toUpperCase() : e.name.slice(e.name.length - e.ext.length)
    return `${s}.${ext}`
  }

  const BAD = /[\\/:*?"<>|]/
  const rows = $derived.by(() => {
    const out = sorted.map((e, i) => ({ e, name: build(e, i), problem: '' }))
    const count = new Map<string, number>()
    for (const r of out) count.set(r.name.toLowerCase(), (count.get(r.name.toLowerCase()) ?? 0) + 1)
    for (const r of out) {
      if (!r.name || r.name.startsWith('.')) r.problem = 'Pusta nazwa'
      else if (BAD.test(r.name)) r.problem = 'Niedozwolony znak'
      else if ((count.get(r.name.toLowerCase()) ?? 0) > 1) r.problem = 'Powtarza się'
    }
    return out
  })
  const changed = $derived(rows.filter((r) => r.name !== r.e.name))
  const problems = $derived(rows.filter((r) => r.problem).length)
  let regexErr = $derived.by(() => {
    if (!regex || !find) return ''
    try {
      new RegExp(find)
      return ''
    } catch (e) {
      return 'Błędne wyrażenie regularne'
    }
  })

  function insert(tok: string) {
    pattern = pattern + (pattern && !pattern.endsWith(' ') && !pattern.endsWith('_') ? '_' : '') + tok
  }

  async function apply() {
    if (!changed.length || problems) return
    busy = true
    try {
      const res = await api.renameMany(changed.map((r) => [r.e.path, (parent(r.e.path) ?? '').replace(/\\?$/, '\\') + r.name] as [string, string]))
      if (res.errors.length) toast(res.errors.slice(0, 3).join('\n'), 'error')
      else {
        toast(`Zmieniono nazwy: ${res.done}`, 'ok', { label: 'Cofnij', run: () => api.undo().then((m) => { toast(m, 'ok'); load(activeTab(), true) }) })
        app.dialog = null
      }
      load(activeTab(), true)
    } catch (e) {
      toast(String(e), 'error')
    }
    busy = false
  }
</script>

<div class="backdrop fade-in" role="presentation" onmousedown={(e) => e.target === e.currentTarget && (app.dialog = null)}>
  <form class="dlg" onsubmit={(e) => { e.preventDefault(); apply() }}>
    <header class="row">
      <span class="hi"><Icon name="edit" size={18} /></span>
      <div class="grow"><h2>Zmień nazwy</h2><p class="faint">{entries.length} elementów · podgląd aktualizuje się na bieżąco</p></div>
    </header>

    <div class="cols">
      <div class="side">
        <label class="field"><span>Wzorzec nazwy</span>
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:value={pattern} spellcheck="false" autofocus />
        </label>
        <div class="tokens">
          {#each TOKENS as [t, hint] (t)}<button type="button" class="chip" title={hint} onclick={() => insert(t)}>{t}</button>{/each}
        </div>
        <div class="tokens">
          {#each PRESETS as [l, p] (l)}<button type="button" class="chip ghost" class:on={pattern === p} onclick={() => (pattern = p)}>{l}</button>{/each}
        </div>

        <div class="two">
          <label class="field"><span>Numer od</span><input type="number" min="0" bind:value={start} /></label>
          <label class="field"><span>Cyfr</span><input type="number" min="1" max="8" bind:value={pad} /></label>
        </div>
        <label class="field"><span>Kolejność numeracji</span>
          <select bind:value={order}>
            <option value="list">Jak na liście</option>
            <option value="name">Po nazwie</option>
            <option value="taken">Po dacie wykonania</option>
            <option value="modified">Po dacie modyfikacji</option>
          </select>
        </label>

        <div class="two">
          <label class="field"><span>Znajdź</span><input bind:value={find} spellcheck="false" placeholder="np. DSC0" /></label>
          <label class="field"><span>Zamień na</span><input bind:value={replace} spellcheck="false" /></label>
        </div>
        <div class="row opts">
          <button type="button" class="chip" class:on={regex} onclick={() => (regex = !regex)} title="Wyrażenie regularne">.*</button>
          <button type="button" class="chip" class:on={matchCase} onclick={() => (matchCase = !matchCase)} title="Uwzględniaj wielkość liter">Aa</button>
          {#if regexErr}<span class="err">{regexErr}</span>{/if}
        </div>

        <div class="two">
          <label class="field"><span>Wielkość liter</span>
            <select bind:value={caseMode}>
              <option value="keep">Bez zmian</option>
              <option value="lower">małe litery</option>
              <option value="upper">WIELKIE LITERY</option>
              <option value="title">Jak W Tytule</option>
            </select>
          </label>
          <label class="field"><span>Rozszerzenie</span>
            <select bind:value={extMode}>
              <option value="keep">Bez zmian</option>
              <option value="lower">.jpg</option>
              <option value="upper">.JPG</option>
            </select>
          </label>
        </div>
      </div>

      <div class="preview">
        <div class="ph row"><span class="grow">Podgląd</span><span class="faint">{changed.length} zmian{problems ? ` · ${problems} problemów` : ''}</span></div>
        <div class="list">
          {#each rows as r (r.e.path)}
            <div class="pr" class:same={r.name === r.e.name} class:bad={!!r.problem} title={r.problem}>
              <span class="old ellipsis">{r.e.name}</span>
              <Icon name="chevron" size={12} />
              <span class="new ellipsis">{r.name}</span>
              {#if r.problem}<span class="tag">{r.problem}</span>{/if}
            </div>
          {/each}
        </div>
      </div>
    </div>

    <div class="btns">
      <span class="faint grow small">Ctrl+Z cofa całą operację</span>
      <button type="button" class="btn ghost" onclick={() => (app.dialog = null)}>Anuluj</button>
      <button class="btn primary" type="submit" disabled={busy || !changed.length || !!problems}>{busy ? 'Zmieniam…' : `Zmień ${changed.length || ''}`}</button>
    </div>
  </form>
</div>

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
    width: 980px;
    max-width: 100%;
    height: min(680px, 100%);
    display: flex;
    flex-direction: column;
    gap: 14px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 20px;
    padding: 20px 22px 16px;
    box-shadow: 0 40px 100px var(--shadow);
  }
  header {
    gap: 12px;
  }
  .hi {
    width: 40px;
    height: 40px;
    border-radius: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--accent);
    color: var(--on-accent);
  }
  h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }
  header p {
    margin: 2px 0 0;
    font-size: 12px;
  }
  .cols {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 340px 1fr;
    gap: 16px;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow-y: auto;
    padding-right: 4px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
  }
  .field span {
    font-size: 12px;
    color: var(--faint);
  }
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .tokens {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .chip {
    font-family: var(--mono);
    font-size: 11px;
  }
  .chip.ghost {
    font-family: var(--font);
  }
  .opts {
    gap: 6px;
  }
  .err {
    color: var(--rec);
    font-size: 12px;
  }
  .preview {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-radius: 14px;
    background: var(--bg);
    outline: 1px solid var(--line);
    overflow: hidden;
  }
  .ph {
    padding: 10px 14px;
    font-size: 12px;
    font-weight: 600;
    color: var(--dim);
    border-bottom: 1px solid var(--line);
  }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 6px;
  }
  .pr {
    display: grid;
    grid-template-columns: 1fr 14px 1fr auto;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 8px;
    border-radius: 8px;
    font-size: 12.5px;
    color: var(--faint);
  }
  .pr:hover {
    background: var(--hover);
  }
  .pr .new {
    color: var(--text);
    font-weight: 500;
  }
  .pr.same .new {
    color: var(--faint);
    font-weight: 400;
  }
  .pr.bad .new {
    color: var(--rec);
  }
  .tag {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--rec) 18%, transparent);
    color: var(--rec);
  }
  .btns {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .small {
    font-size: 12px;
  }
</style>
