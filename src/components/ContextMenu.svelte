<script lang="ts">
  import Icon from './Icon.svelte'
  import ContextMenu from './ContextMenu.svelte'
  import { app, type MenuItem } from '../lib/state.svelte'
  import type { IconName } from '../lib/icons'

  let { items, x, y, sub = false, onclose }: { items: MenuItem[]; x: number; y: number; sub?: boolean; onclose: () => void } = $props()

  let el = $state<HTMLDivElement>()
  let pos = $state({ x: 0, y: 0 })
  let active = $state(-1)
  let openSub = $state<{ i: number; x: number; y: number } | null>(null)

  $effect(() => {
    if (!el) return
    const r = el.getBoundingClientRect()
    let nx = x, ny = y
    if (nx + r.width > window.innerWidth - 8) nx = sub ? x - r.width - 220 + 8 : window.innerWidth - r.width - 8
    if (ny + r.height > window.innerHeight - 8) ny = Math.max(8, window.innerHeight - r.height - 8)
    pos = { x: Math.max(8, nx), y: ny }
  })

  const actionable = (m: MenuItem) => !('sep' in m) && !('disabled' in m && m.disabled)

  function run(m: MenuItem, i: number, ev?: MouseEvent) {
    if ('sep' in m) return
    if ('sub' in m) {
      const r = (ev?.currentTarget as HTMLElement | undefined)?.getBoundingClientRect()
      openSub = { i, x: (r?.right ?? pos.x + 220) - 4, y: (r?.top ?? pos.y) - 6 }
      return
    }
    if (m.disabled) return
    app.menu = null
    onclose()
    m.run()
  }

  function onKey(ev: KeyboardEvent) {
    if (sub && !openSub && ev.key === 'ArrowLeft') return onclose()
    if (openSub) return
    const n = items.length
    if (ev.key === 'Escape') {
      onclose()
    } else if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
      const d = ev.key === 'ArrowDown' ? 1 : -1
      let i = active
      for (let k = 0; k < n; k++) {
        i = (i + d + n) % n
        if (actionable(items[i])) break
      }
      active = i
    } else if ((ev.key === 'Enter' || ev.key === 'ArrowRight') && active >= 0) {
      const m = items[active]
      if (ev.key === 'Enter' || 'sub' in m) {
        const btn = el?.querySelectorAll('button')[active] as HTMLElement | undefined
        if ('sub' in m) {
          const r = btn?.getBoundingClientRect()
          openSub = { i: active, x: (r?.right ?? 0) - 4, y: (r?.top ?? 0) - 6 }
        } else run(m, active)
      }
    } else return
    ev.preventDefault()
    ev.stopPropagation()
  }
</script>

<svelte:window onkeydowncapture={onKey} />

<div class="menu fade-in" bind:this={el} style:left="{pos.x}px" style:top="{pos.y}px" role="menu">
  {#each items as m, i}
    {#if 'sep' in m}
      <div class="sep"></div>
    {:else}
      <button
        class:danger={'danger' in m && m.danger}
        class:active={active === i || openSub?.i === i}
        disabled={'disabled' in m && m.disabled}
        role="menuitem"
        onmouseenter={(ev) => {
          active = i
          if ('sub' in m) run(m, i, ev)
          else openSub = null
        }}
        onclick={(ev) => run(m, i, ev)}
      >
        <span class="i">{#if m.icon}<Icon name={m.icon as IconName} size={16} />{/if}</span>
        <span class="grow ellipsis">{m.label}</span>
        {#if 'kbd' in m && m.kbd}<span class="kbd">{m.kbd}</span>{/if}
        {#if 'sub' in m}<Icon name="chevron" size={14} />{/if}
      </button>
    {/if}
  {/each}
</div>

{#if openSub}
  {@const s = items[openSub.i]}
  {#if 'sub' in s}
    <ContextMenu items={s.sub} x={openSub.x} y={openSub.y} sub onclose={() => (openSub = null)} />
  {/if}
{/if}

<style>
  .menu {
    position: fixed;
    z-index: 500;
    min-width: 220px;
    max-width: 380px;
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    padding: 6px;
    background: var(--high);
    border: 1px solid var(--line);
    border-radius: var(--r);
    box-shadow: 0 24px 60px var(--shadow);
  }
  button {
    width: 100%;
    height: 32px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px;
    border-radius: 8px;
    color: var(--dim);
    text-align: left;
  }
  button.active {
    background: var(--press);
    color: var(--text);
  }
  button.danger {
    color: #ff7a82;
  }
  button:disabled {
    opacity: 0.4;
  }
  .i {
    width: 16px;
    display: flex;
  }
  .sep {
    height: 1px;
    background: var(--line);
    margin: 5px 6px;
  }
</style>
