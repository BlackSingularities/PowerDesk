<script lang="ts">
  import Icon from './Icon.svelte'
  import { app, navigate, newTab, refreshSync, confirm, toast, type Tab } from '../lib/state.svelte'
  import { api, type SyncProfile, type SyncStatus } from '../lib/api'
  import { ago, size, dateFull } from '../lib/util'

  let { tab }: { tab: Tab } = $props()
  let openHistory = $state<string | null>(null)

  $effect(() => {
    refreshSync()
    // Odświeżanie względnych czasów („3 min temu”).
    const t = setInterval(() => (now = Date.now()), 30000)
    return () => clearInterval(t)
  })
  let now = $state(Date.now())

  const MODE: Record<string, string> = { mirror: 'Kopia lustrzana', twoway: 'Dwukierunkowa', backup: 'Kopia zapasowa' }

  function stateText(p: SyncProfile, s: SyncStatus | undefined) {
    now
    if (!p.enabled) return 'Wstrzymana'
    if (!s) return 'Oczekuje'
    switch (s.state) {
      case 'scanning': return 'Porównuję foldery…'
      case 'syncing': return `Synchronizuję · ${Math.round(s.progress * 100)}%`
      case 'confirm': return 'Wymaga potwierdzenia'
      case 'error': return 'Błąd'
      case 'offline': return 'Cel niepodłączony'
    }
    return s.connected ? 'Aktualna' : 'Cel niepodłączony'
  }

  function dot(p: SyncProfile, s?: SyncStatus) {
    if (!p.enabled) return ''
    if (s?.state === 'syncing' || s?.state === 'scanning') return 'busy'
    if (s?.state === 'error' || s?.state === 'confirm') return 'rec'
    return s?.connected ? 'ok' : ''
  }

  async function remove(p: SyncProfile) {
    if (!(await confirm('Usunąć synchronizację?', `„${p.name}” przestanie działać. Pliki w obu folderach zostaną nietknięte.`, 'Usuń', true))) return
    await api.syncDelete(p.id)
    refreshSync()
    toast('Usunięto synchronizację')
  }

  async function toggle(p: SyncProfile) {
    await api.syncEnable(p.id, !p.enabled)
    refreshSync()
  }
</script>

<div class="sync">
  <div class="head">
    <div class="grow">
      <h1>Synchronizacja</h1>
      <p class="muted">Foldery trzymane w zgodzie z dyskiem zewnętrznym lub sieciowym. Działa w tle, także po zamknięciu okna (ikona w zasobniku).</p>
    </div>
    <button class="btn" onclick={() => api.syncNow()} disabled={!app.syncProfiles.length}><Icon name="sync" size={16} />Synchronizuj wszystkie</button>
    <button class="btn primary" onclick={() => (app.dialog = { type: 'sync', profile: null })}><Icon name="plus" size={16} />Nowa</button>
  </div>

  {#each app.syncProfiles as p (p.id)}
    {@const s = app.syncStatus[p.id]}
    <article class="card" class:paused={!p.enabled}>
      <div class="top">
        <span class="dot {dot(p, s)}"></span>
        <div class="grow">
          <h2>{p.name}</h2>
          <p class="faint">{MODE[p.mode]}{p.continuous ? ' · na bieżąco' : ''}{p.versioning ? ' · wersjonowanie' : ''}</p>
        </div>
        <span class="state" class:err={s?.state === 'error' || s?.state === 'confirm'}>{stateText(p, s)}</span>
      </div>

      <div class="route">
        <button class="loc" onclick={() => navigate(tab, p.source)} onauxclick={() => newTab(p.source)} title={p.source}>
          <Icon name="folder" size={16} /><span class="ellipsis">{p.source}</span>
        </button>
        <span class="arrow">{p.mode === 'twoway' ? '⇄' : '→'}</span>
        <button class="loc" class:off={!s?.connected} disabled={!s?.connected} onclick={() => s?.resolved_target && navigate(tab, s.resolved_target)} title={s?.resolved_target || p.target.path}>
          <Icon name={p.target.network ? 'network' : 'usb'} size={16} />
          <span class="ellipsis">{s?.connected ? s.resolved_target : p.target.path}</span>
          {#if p.target.label}<span class="faint lbl ellipsis">{p.target.label}</span>{/if}
        </button>
      </div>

      {#if s?.state === 'syncing' || s?.state === 'scanning'}
        <div class="bar"><i style:width="{Math.max(3, s.progress * 100)}%"></i></div>
        <p class="faint small ellipsis">{s.current}</p>
      {/if}

      {#if s?.state === 'confirm' && s.needs_confirm}
        <div class="warn">
          <Icon name="shield" size={20} />
          <div class="grow">
            <b>Wstrzymano dla bezpieczeństwa</b>
            <p class="small">Ten przebieg usunąłby {s.needs_confirm.del_target + s.needs_confirm.del_source} elementów. Jeśli to zamierzone (np. porządki w folderze) – potwierdź. Jeżeli nie, sprawdź, czy właściwy dysk jest podłączony.</p>
          </div>
          <button class="btn" onclick={() => (app.dialog = { type: 'syncPreview', id: p.id })}>Pokaż plan</button>
          <button class="btn danger" onclick={() => api.syncNow(p.id, true)}>Potwierdź</button>
        </div>
      {/if}

      <div class="facts">
        <div><span class="faint">Ostatnia synchronizacja</span><b title={s?.last_sync ? dateFull(s.last_sync) : ''}>{ago(s?.last_sync ?? 0)}</b></div>
        <div>
          <span class="faint">{s?.connected ? 'Cel' : 'Cel był dostępny'}</span>
          <b title={s?.last_seen ? dateFull(s.last_seen) : ''}>{s?.connected ? 'podłączony' : s?.last_seen ? ago(s.last_seen) : 'jeszcze nie'}</b>
        </div>
        <div><span class="faint">Wynik</span><b class="ellipsis" title={s?.last_result}>{s?.last_result || '—'}</b></div>
        {#if !s?.connected && (s?.pending ?? 0) > 0}
          <div><span class="faint">Czeka na przeniesienie</span><b>{s?.pending} zmian</b></div>
        {/if}
        {#if s?.last_counts?.bytes}
          <div><span class="faint">Przesłano ostatnio</span><b>{size(s.last_counts.bytes)}</b></div>
        {/if}
      </div>
      {#if s?.state === 'error' && s.current}<p class="err small">{s.current}</p>{/if}

      <div class="actions">
        {#if s?.state === 'syncing' || s?.state === 'scanning'}
          <button class="btn" onclick={() => api.syncCancel(p.id)}><Icon name="close" size={15} />Przerwij</button>
        {:else}
          <button class="btn primary" disabled={!s?.connected || !p.enabled} onclick={() => api.syncNow(p.id)}><Icon name="sync" size={15} />Synchronizuj</button>
        {/if}
        <button class="btn" disabled={!s?.connected} onclick={() => (app.dialog = { type: 'syncPreview', id: p.id })}><Icon name="eye" size={15} />Podgląd zmian</button>
        <button class="btn" onclick={() => toggle(p)}><Icon name={p.enabled ? 'pause' : 'play'} size={15} />{p.enabled ? 'Wstrzymaj' : 'Wznów'}</button>
        <button class="btn" onclick={() => (app.dialog = { type: 'sync', profile: p })}><Icon name="edit" size={15} />Edytuj</button>
        <button class="btn ghost" onclick={() => (openHistory = openHistory === p.id ? null : p.id)}><Icon name="history" size={15} />Historia</button>
        <span class="grow"></span>
        <button class="icon-btn" title="Usuń" onclick={() => remove(p)}><Icon name="trash" /></button>
      </div>

      {#if openHistory === p.id}
        <div class="hist fade-in">
          {#each s?.history ?? [] as h (h.at)}
            <div class="row"><span class="dot {h.ok ? 'ok' : 'rec'}"></span><span class="faint when">{dateFull(h.at)}</span><span class="ellipsis grow">{h.summary}</span></div>
          {:else}<p class="faint small">Brak historii</p>{/each}
        </div>
      {/if}
    </article>
  {:else}
    <div class="empty">
      <div class="big"><Icon name="sync" size={34} /></div>
      <h2>Zsynchronizuj folder z dyskiem</h2>
      <p class="muted">Wybierz folder na komputerze i dysk zewnętrzny albo udział sieciowy. Gdy dysk jest podłączony, zmiany przenoszą się same. Gdy go nie ma, zobaczysz, kiedy był ostatnio i ile zmian czeka.</p>
      <button class="btn primary" onclick={() => (app.dialog = { type: 'sync', profile: null })}><Icon name="plus" size={16} />Utwórz synchronizację</button>
    </div>
  {/each}
</div>

<style>
  .sync {
    flex: 1;
    overflow-y: auto;
    padding: 16px 24px 40px;
  }
  .head {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    max-width: 1000px;
    margin-bottom: 18px;
  }
  h1 {
    margin: 0 0 4px;
    font-size: 22px;
    font-weight: 600;
  }
  .head p {
    margin: 0;
    max-width: 640px;
  }
  .card {
    max-width: 1000px;
    background: var(--panel);
    border-radius: var(--r-xl);
    padding: 18px 20px;
    margin-bottom: 12px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .card.paused {
    opacity: 0.6;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .top .dot {
    width: 10px;
    height: 10px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .top p {
    margin: 2px 0 0;
    font-size: 12px;
  }
  .state {
    font-size: 12px;
    color: var(--dim);
    background: var(--high);
    padding: 5px 12px;
    border-radius: 999px;
  }
  .state.err {
    background: rgba(255, 59, 71, 0.16);
    color: #ff8a91;
  }
  .route {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .loc {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    background: var(--high);
    border-radius: var(--r);
    padding: 10px 14px;
    color: var(--dim);
    text-align: left;
  }
  .loc:hover:not(:disabled) {
    background: var(--higher);
    color: var(--text);
  }
  .loc.off {
    opacity: 0.55;
  }
  .lbl {
    font-size: 11px;
    margin-left: auto;
    max-width: 40%;
  }
  .arrow {
    font-size: 18px;
    color: var(--faint);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 10px 28px;
  }
  .facts div {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    max-width: 320px;
  }
  .facts span {
    font-size: 11px;
  }
  .facts b {
    font-weight: 500;
  }
  .small {
    font-size: 12px;
    margin: 0;
  }
  .err {
    color: #ff8a91;
  }
  .warn {
    display: flex;
    align-items: center;
    gap: 14px;
    background: rgba(255, 181, 71, 0.1);
    color: var(--warn);
    border-radius: var(--r-l);
    padding: 12px 14px;
  }
  .warn b {
    color: var(--text);
  }
  .warn p {
    color: var(--dim);
    margin-top: 2px;
  }
  .actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
  }
  .actions .btn {
    height: 30px;
    font-size: 12px;
    padding: 0 12px;
  }
  .hist {
    background: var(--bg);
    border-radius: var(--r);
    padding: 10px 14px;
    max-height: 240px;
    overflow-y: auto;
    font-size: 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .when {
    width: 260px;
    flex: none;
  }
  .empty {
    max-width: 520px;
    margin: 60px auto;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }
  .big {
    width: 72px;
    height: 72px;
    border-radius: var(--r-xl);
    background: var(--high);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .empty p {
    margin: 0 0 8px;
  }
</style>
