<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { Terminal } from '@xterm/xterm'
  import { FitAddon } from '@xterm/addon-fit'
  import '@xterm/xterm/css/xterm.css'
  import Icon from './Icon.svelte'
  import { settings, activeTab, toast } from '../lib/state.svelte'
  import { on } from '../lib/api'
  import { isRemote, inArchive } from '../lib/util'

  let { hidden = false }: { hidden?: boolean } = $props()

  let host = $state<HTMLDivElement>()
  let id = $state<number | null>(null)
  let follow = $state(true)
  let cwd = $state('')
  let term: Terminal
  let fit: FitAddon

  const local = (p: string) => !!p && !p.startsWith('ofp:') && !isRemote(p) && !inArchive(p + '\\')
  const tabPath = $derived(activeTab()?.path ?? '')

  function css(name: string) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  }
  function theme() {
    return { background: css('--bg') || '#000', foreground: css('--text') || '#fff', cursor: css('--accent') || '#fff', selectionBackground: css('--sel-strong') }
  }

  async function start() {
    if (id !== null) await invoke('term_kill', { id }).catch(() => {})
    term.clear()
    cwd = local(tabPath) ? tabPath : ''
    try {
      id = await invoke<number>('term_start', { cwd, cols: term.cols, rows: term.rows })
    } catch (e) {
      toast(`Nie udało się uruchomić terminala: ${e}`, 'error')
    }
  }

  onMount(() => {
    term = new Terminal({ fontFamily: css('--mono'), fontSize: 13, cursorBlink: true, theme: theme(), allowProposedApi: false, scrollback: 5000 })
    fit = new FitAddon()
    term.loadAddon(fit)
    term.open(host!)
    fit.fit()
    term.onData((d) => id !== null && invoke('term_write', { id, data: d }).catch(() => {}))
    term.onResize(({ cols, rows }) => id !== null && invoke('term_resize', { id, cols, rows }))
    const offs = [
      on<{ id: number; data: string }>('term-data', (p) => p.id === id && term.write(p.data)),
      on<number>('term-exit', (x) => {
        if (x === id) {
          id = null
          term.write('\r\n\x1b[2m[proces zakończony – Enter uruchomi nowy]\x1b[0m\r\n')
        }
      }),
    ]
    term.onKey(({ domEvent }) => id === null && domEvent.key === 'Enter' && start())
    const ro = new ResizeObserver(() => !hidden && fit.fit())
    ro.observe(host!)
    // Motyw zmienia się razem z aplikacją.
    const mo = new MutationObserver(() => (term.options.theme = theme()))
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'style'] })
    start()
    return () => {
      ro.disconnect()
      mo.disconnect()
      offs.forEach((o) => o.then((f) => f()))
      if (id !== null) invoke('term_kill', { id })
      term.dispose()
    }
  })

  // Terminal idzie za bieżącym folderem aplikacji.
  $effect(() => {
    const p = tabPath
    if (!follow || id === null || !local(p) || p === cwd) return
    cwd = p
    invoke('term_cd', { id, path: p }).catch(() => {})
  })
  $effect(() => {
    if (!hidden) queueMicrotask(() => { fit?.fit(); term?.focus() })
  })

  let drag = false
  function resize(e: PointerEvent) {
    if (drag) settings.terminalHeight = Math.max(120, Math.min(window.innerHeight - 200, window.innerHeight - e.clientY - 26))
  }
</script>

<section class="term" class:hidden style:height="{settings.terminalHeight}px" aria-label="Terminal">
  <div class="grip" role="separator" aria-orientation="horizontal" onpointerdown={(e) => { drag = true; (e.target as HTMLElement).setPointerCapture(e.pointerId) }} onpointermove={resize} onpointerup={() => (drag = false)}></div>
  <header>
    <Icon name="terminal" size={14} />
    <span class="ellipsis grow faint">{cwd || 'Terminal'}</span>
    <button class="chip" class:on={follow} title="Przechodź do folderu otwartego w aplikacji" onclick={() => (follow = !follow)}>Za folderem</button>
    <button class="icon-btn xs" title="Uruchom ponownie" aria-label="Uruchom ponownie" onclick={start}><Icon name="refresh" size={13} /></button>
    <button class="icon-btn xs" title="Ukryj (Ctrl+`)" aria-label="Ukryj terminal" onclick={() => (settings.terminalOpen = false)}><Icon name="close" size={13} /></button>
  </header>
  <div class="host" bind:this={host}></div>
</section>

<style>
  .term {
    position: relative;
    flex: none;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
    background: var(--bg);
  }
  .term.hidden {
    display: none;
  }
  .grip {
    position: absolute;
    top: -3px;
    left: 0;
    right: 0;
    height: 6px;
    cursor: row-resize;
    z-index: 2;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px 4px 12px;
    font-size: 12px;
    flex: none;
  }
  .icon-btn.xs {
    width: 24px;
    height: 24px;
  }
  .chip {
    height: 22px;
    font-size: 11px;
  }
  .host {
    flex: 1;
    min-height: 0;
    padding: 0 0 4px 10px;
  }
  .host :global(.xterm) {
    height: 100%;
  }
</style>
