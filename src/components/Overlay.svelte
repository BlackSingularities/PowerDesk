<script lang="ts">
  import Icon from './Icon.svelte'
  import { app } from '../lib/state.svelte'
  import { api, type OpProgress } from '../lib/api'
  import { size } from '../lib/util'

  function eta(o: OpProgress) {
    if (!o.speed || !o.total_bytes) return ''
    const s = (o.total_bytes - o.done_bytes) / o.speed
    if (s < 60) return `${Math.max(1, Math.round(s))} s`
    if (s < 3600) return `${Math.round(s / 60)} min`
    return `${(s / 3600).toFixed(1)} h`
  }
  let openErrors = $state<number | null>(null)
</script>

<div class="stack">
  {#each app.ops as o (o.id)}
    {@const pct = o.total_bytes ? o.done_bytes / o.total_bytes : o.total_items ? o.done_items / o.total_items : 0}
    <div class="op fade-in" class:err={o.state === 'error'}>
      <div class="row">
        <Icon name={o.kind === 'copy' ? 'copy' : o.kind === 'move' ? 'cut' : 'trash'} size={16} />
        <b class="grow ellipsis">{o.title}</b>
        {#if o.state === 'running'}
          <button class="icon-btn s" title="Anuluj" onclick={() => api.cancelOp(o.id)}><Icon name="close" size={14} /></button>
        {:else if o.state === 'error'}
          <button class="link" onclick={() => (openErrors = openErrors === o.id ? null : o.id)}>{o.errors.length} błędów</button>
        {:else}
          <Icon name="check" size={16} />
        {/if}
      </div>
      <div class="bar"><i style:width="{Math.min(100, pct * 100)}%"></i></div>
      <div class="meta faint">
        <span class="ellipsis grow">{o.current || (o.state === 'done' ? 'Gotowe' : o.state === 'cancelled' ? 'Anulowano' : '')}</span>
        {#if o.state === 'running' && o.total_bytes}<span>{size(o.done_bytes)} / {size(o.total_bytes)} · {size(o.speed)}/s · {eta(o)}</span>
        {:else if o.state === 'running'}<span>{o.done_items} / {o.total_items}</span>{/if}
      </div>
      {#if openErrors === o.id}
        <div class="errs">{#each o.errors as e}<div>{e}</div>{/each}</div>
      {/if}
    </div>
  {/each}

  {#each app.toasts as t (t.id)}
    <div class="toast fade-in {t.kind}" role={t.kind === 'error' ? 'alert' : 'status'} aria-live={t.kind === 'error' ? 'assertive' : 'polite'}>
      {#if t.kind === 'ok'}<Icon name="check" size={16} />{:else if t.kind === 'error'}<Icon name="info" size={16} />{/if}
      <span class="grow">{t.text}</span>
      {#if t.action}
        <button class="act" onclick={() => { t.action!.run(); app.toasts = app.toasts.filter((x) => x.id !== t.id) }}>{t.action.label}</button>
      {/if}
    </div>
  {/each}
</div>

<style>
  .stack {
    position: fixed;
    right: 18px;
    bottom: 40px;
    z-index: 400;
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 380px;
    pointer-events: none;
  }
  .op,
  .toast {
    pointer-events: auto;
    background: var(--popup-bg);
    backdrop-filter: var(--popup-blur);
    border: 1px solid var(--line);
    border-radius: var(--r-l);
    box-shadow: 0 18px 50px var(--shadow);
  }
  .op {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .op.err {
    border-color: rgba(255, 59, 71, 0.5);
  }
  .meta {
    display: flex;
    gap: 10px;
    font-size: 11.5px;
  }
  .s {
    width: 24px;
    height: 24px;
  }
  .errs {
    max-height: 140px;
    overflow-y: auto;
    font-size: 11.5px;
    color: #ff8a91;
    user-select: text;
  }
  .link {
    font-size: 12px;
    color: #ff8a91;
    text-decoration: underline;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 11px 14px;
    color: var(--text);
  }
  .toast.ok :global(.ico) {
    color: var(--ok);
  }
  .toast.error :global(.ico) {
    color: var(--rec);
  }
  .act {
    font-weight: 600;
    padding: 4px 10px;
    border-radius: var(--r-s);
    background: var(--accent);
    color: var(--on-accent);
  }
</style>
