<script lang="ts">
  import Icon from './Icon.svelte'
  import logo from '../../app-icon.svg'
  import type { IconName } from '../lib/icons'
  import { app, settings, toast, updateNow } from '../lib/state.svelte'
  import { api } from '../lib/api'
  import { getVersion } from '@tauri-apps/api/app'

  let autostart = $state(false)
  let version = $state('')
  let checking = $state(false)
  let q = $state('')
  let active = $state('browse')
  let scroller: HTMLDivElement
  api.getAutostart().then((v) => (autostart = v))
  let integ = $state<{ installed: boolean; default_fm: boolean; context_menu: boolean } | null>(null)
  const refreshInteg = () => api.integration().then((v) => (integ = v)).catch(() => {})
  refreshInteg()
  getVersion().then((v) => (version = v))

  type Key = { [K in keyof typeof settings]: (typeof settings)[K] extends boolean ? K : never }[keyof typeof settings]
  type Special = 'autostart' | 'defaultFm' | 'ctxMenu' | `sb:${keyof typeof settings.sidebar}`
  type Opt = { key: Key | Special; icon: IconName; label: string; hint?: string; kbd?: string }
  type Group = { id: string; title: string; icon: IconName; lead: string; items: Opt[] }

  const groups: Group[] = [
    {
      id: 'browse', title: 'Przeglądanie', icon: 'folder', lead: 'Jak wyglądają i otwierają się foldery',
      items: [
        { key: 'showHidden', icon: 'eye', label: 'Ukryte i systemowe pliki', kbd: 'Ctrl+H' },
        { key: 'foldersFirst', icon: 'sort', label: 'Foldery przed plikami' },
        { key: 'singleClickOpen', icon: 'bolt', label: 'Otwieraj jednym kliknięciem' },
        { key: 'rememberViewPerFolder', icon: 'history', label: 'Pamiętaj widok każdego folderu', hint: 'Widok, sortowanie i grupowanie' },
        { key: 'autoGallery', icon: 'gallery', label: 'Zdjęcia i filmy w galerii', hint: 'Foldery z mediami otwierają się w galerii' },
        { key: 'folderSizes', icon: 'hash', label: 'Rozmiary folderów', hint: 'Liczone w tle w kolumnie Rozmiar · bez dysków sieciowych' },
      ],
    },
    {
      id: 'nav', title: 'Panel boczny i Komputer', icon: 'sidebar', lead: 'Co widać w nawigacji',
      items: [
        { key: 'showQuickAccess', icon: 'star', label: 'Szybki dostęp', hint: 'Sekcja w panelu bocznym' },
        { key: 'sb:showSaved', icon: 'search', label: 'Zapisane wyszukiwania', hint: 'Sekcja w panelu bocznym' },
        { key: 'sb:showFav', icon: 'pin', label: 'Przypięte', hint: 'Sekcja w panelu bocznym' },
        { key: 'sb:showLibs', icon: 'archive', label: 'Biblioteki', hint: 'Biblioteki Windows – nowe są widoczne też w Eksploratorze' },
        { key: 'sb:showDrives', icon: 'drive', label: 'Dyski', hint: 'Sekcja w panelu bocznym' },
        { key: 'sb:showNet', icon: 'network', label: 'Sieć', hint: 'Dyski sieciowe i serwery FTP' },
        { key: 'sb:showSync', icon: 'sync', label: 'Synchronizacja', hint: 'Jedyne miejsce, gdzie dodaje się synchronizacje' },
        { key: 'showRecent', icon: 'history', label: 'Ostatnio otwierane', hint: 'Lista na ekranie Komputer' },
        { key: 'hideOfflineDrives', icon: 'usb', label: 'Ukrywaj puste napędy', hint: 'Czytniki kart bez nośnika, CD/DVD bez płyty' },
      ],
    },
    {
      id: 'preview', title: 'Podgląd', icon: 'panel', lead: 'Panel z prawej i miniatury',
      items: [
        { key: 'previewPane', icon: 'panel', label: 'Panel podglądu', kbd: 'Ctrl+P' },
        { key: 'playVideoPreview', icon: 'play', label: 'Odtwarzaj wideo w podglądzie', hint: 'Bez dźwięku' },
        { key: 'pdfThumbs', icon: 'doc', label: 'Miniatury PDF', hint: 'Gdy Windows ich nie dostarcza · tylko w pamięci' },
      ],
    },
    {
      id: 'safety', title: 'Bezpieczeństwo', icon: 'shield', lead: 'Potwierdzenia przed zmianami',
      items: [{ key: 'confirmTrash', icon: 'trash', label: 'Pytaj przed przeniesieniem do Kosza', hint: 'Usuwanie trwałe zawsze wymaga potwierdzenia' }],
    },
    {
      id: 'bg', title: 'Działanie w tle', icon: 'sync', lead: 'Zasobnik, autostart i aktualizacje',
      items: [
        { key: 'closeToTray', icon: 'min', label: 'Zamknięcie chowa do zasobnika', hint: 'Tylko gdy są synchronizacje – działają dalej' },
        { key: 'autostart', icon: 'bolt', label: 'Uruchamiaj z systemem', hint: 'Synchronizacja od razu po zalogowaniu' },
        { key: 'checkUpdates', icon: 'download', label: 'Sprawdzaj nowe wersje' },
      ],
    },
    {
      id: 'windows', title: 'Integracja z Windows', icon: 'app', lead: 'Otwieranie folderów z innych programów i pulpitu',
      items: [
        { key: 'defaultFm', icon: 'folder', label: 'Domyślny menedżer plików', hint: 'Foldery i dyski otwierają się tutaj · Win+E nadal otwiera Eksplorator' },
        { key: 'ctxMenu', icon: 'more', label: '„Otwórz w OpenFilePower” w menu', hint: 'Pozycja w menu kontekstowym folderów Eksploratora' },
      ],
    },
  ]

  const KEYS: [string, string][] = [
    ['Ctrl+T / Ctrl+W', 'Nowa / zamknij kartę'],
    ['Ctrl+Shift+T', 'Przywróć zamkniętą kartę'],
    ['Ctrl+Tab', 'Następna karta'],
    ['Ctrl+\\', 'Podział na dwa panele'],
    ['Ctrl+K', 'Paleta: foldery, pliki, polecenia'],
    ['Ctrl+Shift+P', 'Paleta poleceń'],
    ['F6 / Shift+F6', 'Następny / poprzedni obszar okna'],
    ['Ctrl+L', 'Edytuj ścieżkę'],
    ['Ctrl+F', 'Szukaj'],
    ['Alt+← / → / ↑', 'Wstecz / dalej / w górę'],
    ['Spacja', 'Podgląd pełnoekranowy'],
    ['F2', 'Zmień nazwę · kilka zaznaczonych – zmiana nazw'],
    ['Del / Shift+Del', 'Kosz / trwale'],
    ['Ctrl+C / X / V', 'Kopiuj / wytnij / wklej'],
    ['Ctrl+Shift+C', 'Kopiuj ścieżkę'],
    ['Ctrl+Z', 'Cofnij'],
    ['Ctrl+Y / Ctrl+Shift+Z', 'Ponów'],
    ['Ctrl+Shift+N', 'Nowy folder'],
    ['Ctrl+1 / 2 / 3', 'Lista / siatka / galeria'],
    ['Ctrl+kółko', 'Wielkość miniatur'],
    ['Alt+Enter', 'Właściwości'],
    ['Ctrl+H', 'Ukryte pliki'],
  ]

  const VIEWS = [
    { id: 'list', label: 'Lista', kbd: 'Ctrl+1' },
    { id: 'grid', label: 'Siatka', kbd: 'Ctrl+2' },
    { id: 'gallery', label: 'Galeria', kbd: 'Ctrl+3' },
  ] as const

  const THEMES = [
    { id: 'system', label: 'Jak w systemie', hint: 'Podąża za ustawieniem Windows' },
    { id: 'dark', label: 'Ciemny', hint: 'Czarne tło, biały akcent' },
    { id: 'light', label: 'Jasny', hint: 'Jasne tło, czarny akcent' },
  ] as const
  const ACCENTS = ['#4f8cff', '#8b5cf6', '#ec4899', '#ef4444', '#f97316', '#eab308', '#22c55e', '#14b8a6']
  const STARTS = [
    { id: 'restore', icon: 'history', label: 'Ostatnie karty', hint: 'Wszystko tak, jak zostawiłeś' },
    { id: 'home', icon: 'pc', label: 'Komputer', hint: 'Jedna karta z ekranem Komputer' },
    { id: 'folder', icon: 'folder', label: 'Wybrany folder', hint: 'Zawsze ten sam folder' },
  ] as const
  async function pickStart() {
    const p = await api.pickFolder(settings.startupFolder || undefined)
    if (p) {
      settings.startupFolder = p
      settings.startup = 'folder'
    }
  }

  const NAV: { id: string; title: string; icon: IconName }[] = [
    { id: 'look', title: 'Wygląd', icon: 'eye' },
    { id: 'start', title: 'Po uruchomieniu', icon: 'bolt' },
    { id: 'view', title: 'Domyślny widok', icon: 'grid' },
    ...groups.map((g) => ({ id: g.id, title: g.title, icon: g.icon })),
    { id: 'keys', title: 'Skróty klawiszowe', icon: 'hash' },
    { id: 'about', title: 'O aplikacji', icon: 'info' },
  ]

  const norm = (s: string) => s.toLocaleLowerCase('pl')
  const match = (...s: (string | undefined)[]) => !q.trim() || s.some((x) => x && norm(x).includes(norm(q.trim())))
  const shown = $derived(groups.map((g) => ({ ...g, items: g.items.filter((o) => match(o.label, o.hint, g.title)) })).filter((g) => g.items.length))
  const keysShown = $derived(KEYS.filter(([k, l]) => match(k, l, 'skróty klawiszowe')))
  const viewShown = $derived(match('domyślny widok lista siatka galeria'))
  const lookShown = $derived(match('wygląd motyw jasny ciemny systemowy krycie przezroczystość okienka rozmycie'))
  const startShown = $derived(match('po uruchomieniu start karty komputer folder'))
  const onCount = $derived(groups.flatMap((g) => g.items).filter((o) => val(o)).length)

  const val = (o: Opt) =>
    o.key.startsWith('sb:') ? settings.sidebar[o.key.slice(3) as keyof typeof settings.sidebar] : o.key === 'autostart' ? autostart : o.key === 'defaultFm' ? !!integ?.default_fm : o.key === 'ctxMenu' ? !!integ?.context_menu : settings[o.key as Key]
  function flip(o: Opt) {
    if (o.key.startsWith('sb:')) {
      const k = o.key.slice(3) as keyof typeof settings.sidebar
      settings.sidebar[k] = !settings.sidebar[k]
      return
    }
    if (o.key === 'autostart') return toggleAutostart()
    if (o.key === 'defaultFm' || o.key === 'ctxMenu') {
      const on = !val(o)
      const fn = o.key === 'defaultFm' ? api.setDefaultFm : api.setContextMenu
      return fn(on)
        .then(() => {
          toast(o.key === 'defaultFm' ? (on ? 'Foldery będą otwierać się w OpenFilePower' : 'Przywrócono Eksplorator jako domyślny') : on ? 'Dodano pozycję do menu Eksploratora' : 'Usunięto pozycję z menu', 'ok')
          refreshInteg()
        })
        .catch((e) => toast(String(e), 'error'))
    }
    const k = o.key as Key
    settings[k] = !settings[k]
  }
  const caps = (k: string) => k.split(' / ').map((alt) => (alt === '+' ? ['+'] : alt.split('+')))

  function jump(id: string) {
    scroller.querySelector<HTMLElement>(`#s-${id}`)?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    active = id
  }
  function spy() {
    const top = scroller.getBoundingClientRect().top + 80
    let cur = NAV[0].id
    for (const n of NAV) {
      const el = scroller.querySelector<HTMLElement>(`#s-${n.id}`)
      if (el && el.getBoundingClientRect().top <= top) cur = n.id
    }
    active = cur
  }

  async function toggleAutostart() {
    try {
      await api.setAutostart(!autostart)
      autostart = !autostart
    } catch (e) {
      toast(String(e), 'error')
    }
  }

  async function check() {
    checking = true
    try {
      const u = await api.checkUpdate()
      if (u.newer) app.update = { latest: u.latest, url: u.url }
      toast(u.newer ? `Dostępna wersja ${u.latest}` : 'Masz najnowszą wersję', u.newer ? 'info' : 'ok')
    } catch (e) {
      toast(`Nie udało się sprawdzić: ${e}`, 'error')
    }
    checking = false
  }
</script>

<div class="wrap">
  <nav>
    <div class="me">
      <div class="logo">
        <img src={logo} width="34" height="34" alt="" />
      </div>
      <div><b>Ustawienia</b><span class="faint">{onCount} włączonych opcji</span></div>
    </div>
    <label class="find">
      <Icon name="search" size={15} />
      <input placeholder="Znajdź ustawienie…" bind:value={q} />
      {#if q}<button onclick={() => (q = '')} aria-label="Wyczyść"><Icon name="close" size={13} /></button>{/if}
    </label>
    {#each NAV as n (n.id)}
      <button class="nav" class:on={active === n.id && !q} onclick={() => { q = ''; requestAnimationFrame(() => jump(n.id)) }}><Icon name={n.icon} size={16} /><span class="ellipsis">{n.title}</span></button>
    {/each}
  </nav>

  <div class="main" bind:this={scroller} onscroll={spy}>
    {#if lookShown}
      <section id="s-look">
        <header><h2>Wygląd</h2><p>Motyw całej aplikacji</p></header>
        <div class="views">
          {#each THEMES as th (th.id)}
            <button class="view" class:on={settings.theme === th.id} onclick={() => (settings.theme = th.id)}>
              <span class="tmock {th.id}"><span class="half d"><i></i><i></i><i></i></span><span class="half l"><i></i><i></i><i></i></span></span>
              <span class="vl"><b>{th.label}</b>{#if settings.theme === th.id}<span class="tick"><Icon name="check" size={12} /></span>{/if}</span>
              <small class="vh">{th.hint}</small>
            </button>
          {/each}
        </div>
        <div class="slider card">
          <span class="ic"><Icon name="gallery" size={18} /></span>
          <span class="txt"><b>Tło okna</b><small>Mica i Acrylic przepuszczają kolor tapety (Windows 11)</small></span>
          <div class="mini-seg" role="radiogroup" aria-label="Tło okna">
            {#each [['none', 'Pełne'], ['mica', 'Mica'], ['acrylic', 'Acrylic']] as [id, l] (id)}
              <button role="radio" aria-checked={settings.windowEffect === id} class:on={settings.windowEffect === id} onclick={() => (settings.windowEffect = id as typeof settings.windowEffect)}>{l}</button>
            {/each}
          </div>
        </div>
        <div class="slider card">
          <span class="ic"><Icon name="dot" size={18} /></span>
          <span class="txt"><b>Kolor akcentu</b><small>Przyciski, zaznaczenia, przełączniki</small></span>
          <div class="swatches" role="radiogroup" aria-label="Kolor akcentu">
            <button class="sw mono" class:on={settings.accent === 'mono'} role="radio" aria-checked={settings.accent === 'mono'} title="Biały / czarny" aria-label="Biały lub czarny" onclick={() => (settings.accent = 'mono')}></button>
            <button class="sw sys" class:on={settings.accent === 'system'} role="radio" aria-checked={settings.accent === 'system'} title="Jak w Windows" aria-label="Kolor z Windows" onclick={() => (settings.accent = 'system')}>W</button>
            {#each ACCENTS as c (c)}
              <button class="sw" class:on={settings.accent === c} style:background={c} role="radio" aria-checked={settings.accent === c} aria-label={c} onclick={() => (settings.accent = c)}></button>
            {/each}
            <label class="sw pick" title="Własny kolor"><input type="color" value={settings.accent.startsWith('#') ? settings.accent : '#4f8cff'} oninput={(e) => (settings.accent = (e.target as HTMLInputElement).value)} aria-label="Własny kolor" />+</label>
          </div>
        </div>
        <div class="slider card">
          <span class="ic"><Icon name="panel" size={18} /></span>
          <span class="txt"><b>Krycie okienek</b><small>Menu, okna dialogowe, paleta, podpowiedzi i powiadomienia · poniżej 100% tło jest rozmyte</small></span>
          <input type="range" min="60" max="100" step="5" bind:value={settings.popupOpacity} aria-label="Krycie okienek" />
          <span class="pct">{settings.popupOpacity}%</span>
        </div>
      </section>
    {/if}

    {#if startShown}
      <section id="s-start">
        <header><h2>Po uruchomieniu</h2><p>Co zobaczysz po otwarciu aplikacji · przypięte karty wracają zawsze</p></header>
        <div class="cards">
          {#each STARTS as st (st.id)}
            <button class="card" class:on={settings.startup === st.id} role="radio" aria-checked={settings.startup === st.id} onclick={() => (st.id === 'folder' && !settings.startupFolder ? pickStart() : (settings.startup = st.id))}>
              <span class="ic"><Icon name={st.icon} size={18} /></span>
              <span class="txt"><b>{st.label}</b><small class="ellipsis">{st.id === 'folder' && settings.startupFolder ? settings.startupFolder : st.hint}</small></span>
              {#if st.id === 'folder'}<span class="btn ghost sm" role="button" tabindex="-1" onclick={(e) => { e.stopPropagation(); pickStart() }} onkeydown={() => {}}>Wybierz…</span>{/if}
              <span class="radio" class:on={settings.startup === st.id}></span>
            </button>
          {/each}
        </div>
      </section>
    {/if}

    {#if viewShown}
      <section id="s-view">
        <header><h2>Domyślny widok</h2><p>Jak otwiera się folder, dla którego nic nie zapamiętano</p></header>
        <div class="views">
          {#each VIEWS as v (v.id)}
            <button class="view" class:on={settings.defaultView === v.id} onclick={() => (settings.defaultView = v.id)}>
              <span class="mock {v.id}">
                {#if v.id === 'list'}
                  {#each Array(5) as _, i (i)}<i><em></em><s style:width="{[62, 44, 70, 38, 55][i]}%"></s></i>{/each}
                {:else if v.id === 'grid'}
                  {#each Array(8) as _, i (i)}<i></i>{/each}
                {:else}
                  {#each [3, 2, 4, 2, 3, 3] as w, i (i)}<i style:flex-grow={w}></i>{/each}
                {/if}
              </span>
              <span class="vl"><b>{v.label}</b><span class="kbd">{v.kbd}</span>{#if settings.defaultView === v.id}<span class="tick"><Icon name="check" size={12} /></span>{/if}</span>
            </button>
          {/each}
        </div>
      </section>
    {/if}

    {#each shown as g (g.id)}
      <section id="s-{g.id}">
        <header><h2>{g.title}</h2><p>{g.lead}</p></header>
        <div class="cards">
          {#each g.items as o (o.key)}
            <button class="card" class:on={val(o)} onclick={() => flip(o)} role="switch" aria-checked={val(o)}>
              <span class="ic"><Icon name={o.icon} size={18} /></span>
              <span class="txt">
                <b>{o.label}</b>
                {#if o.hint}<small>{o.hint}</small>{/if}
                {#if o.kbd}<span class="caps">{#each o.kbd.split('+') as c (c)}<kbd>{c}</kbd>{/each}</span>{/if}
              </span>
              <span class="switch" class:on={val(o)}></span>
            </button>
          {/each}
        </div>
      </section>
    {/each}

    {#if keysShown.length}
      <section id="s-keys">
        <header><h2>Skróty klawiszowe</h2><p>Zgodne z Eksploratorem Windows tam, gdzie to możliwe</p></header>
        <div class="keys">
          {#each keysShown as [k, l] (k)}
            <div class="key">
              <span class="caps">
                {#each caps(k) as alt, ai (ai)}
                  {#if ai}<span class="or">/</span>{/if}
                  {#each alt as c, ci (ci)}<kbd>{c}</kbd>{/each}
                {/each}
              </span>
              <span class="muted">{l}</span>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if !q}
      <section id="s-about">
        <div class="about">
          <div class="logo big">
            <img src={logo} width="52" height="52" alt="" />
          </div>
          <div class="grow">
            <b>OpenFilePower dla Windows</b>
            <p class="faint">Wersja {version || '…'}{integ ? (integ.installed ? ' · zainstalowana – aktualizuje się sama' : ' · wersja przenośna') : ''} · miniatury i metadane pochodzą z systemu Windows</p>
          </div>
          {#if app.update}
            <button class="btn primary" onclick={updateNow} disabled={app.updating !== null}><Icon name="download" size={15} />{app.updating !== null ? `Pobieram… ${Math.round(app.updating * 100)}%` : integ?.installed ? `Zainstaluj ${app.update.latest}` : `Pobierz ${app.update.latest}`}</button>
          {:else}
            <button class="btn" onclick={check} disabled={checking}><Icon name="refresh" size={15} />{checking ? 'Sprawdzam…' : 'Sprawdź aktualizacje'}</button>
          {/if}
        </div>
      </section>
    {/if}

    {#if q && !shown.length && !keysShown.length && !viewShown && !lookShown && !startShown}
      <div class="empty"><Icon name="search" size={28} /><p>Nic nie pasuje do „{q}”</p></div>
    {/if}
  </div>
</div>

<style>
  .wrap {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  nav {
    width: 240px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 18px 12px;
    overflow-y: auto;
  }
  .me {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 6px 16px;
  }
  .me b {
    display: block;
    font-size: 17px;
    font-weight: 600;
  }
  .me span {
    font-size: 12px;
  }
  .logo {
    width: 40px;
    height: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex: none;
  }
  .logo :global(.lf) {
    fill: var(--accent);
  }
  .logo :global(.ls) {
    stroke: var(--bg);
  }
  .logo.big {
    width: 64px;
    height: 64px;
    border-radius: var(--r-xl);
  }
  .find {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 10px;
    margin-bottom: 12px;
    border-radius: var(--r);
    background: var(--card);
    color: var(--faint);
  }
  .find:focus-within {
    outline: 1px solid var(--line-strong);
    color: var(--text);
  }
  .find input {
    flex: 1;
    min-width: 0;
    background: none;
    border: 0;
    outline: 0;
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .find button {
    display: flex;
    color: var(--faint);
  }
  .nav {
    display: flex;
    align-items: center;
    gap: 11px;
    height: 34px;
    padding: 0 10px;
    border-radius: var(--r);
    color: var(--dim);
    text-align: left;
    transition: background 0.12s, color 0.12s;
  }
  .nav:hover {
    background: var(--hover);
    color: var(--text);
  }
  .nav.on {
    background: var(--sel);
    color: var(--text);
  }

  .main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 22px 28px 60px;
    scroll-padding-top: 16px;
  }
  section {
    max-width: 980px;
    margin-bottom: 34px;
  }
  header {
    margin-bottom: 12px;
  }
  h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }
  header p {
    margin: 3px 0 0;
    color: var(--faint);
    font-size: 12.5px;
  }

  .views {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
  }
  .view {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px;
    border-radius: var(--r-l);
    background: var(--panel);
    outline: 1px solid var(--line);
    text-align: left;
    transition: background 0.12s, outline-color 0.12s;
  }
  .view:hover {
    background: var(--high);
  }
  .view.on {
    outline: 2px solid var(--accent);
    background: var(--high);
  }
  .mock {
    height: 92px;
    border-radius: var(--r);
    background: var(--bg);
    padding: 10px;
    display: flex;
    gap: 5px;
    overflow: hidden;
  }
  .mock i {
    display: block;
    border-radius: 4px;
    background: var(--press);
  }
  .mock.list {
    flex-direction: column;
    gap: 6px;
  }
  .mock.list i {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 10px;
    background: none;
  }
  .mock.list em {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    background: var(--ghost);
  }
  .mock.list s {
    height: 6px;
    border-radius: 3px;
    background: var(--press);
  }
  .mock.grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
  }
  .mock.gallery {
    flex-wrap: wrap;
    align-content: flex-start;
  }
  .mock.gallery i {
    height: 33px;
    flex-basis: 30px;
  }
  .view.on .mock.grid i,
  .view.on .mock.gallery i,
  .view.on .mock.list em {
    background: var(--sel-strong);
  }
  .tmock {
    height: 92px;
    border-radius: var(--r);
    overflow: hidden;
    display: flex;
    outline: 1px solid var(--line);
  }
  .tmock .half {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 10px;
  }
  .tmock .d {
    background: #000;
  }
  .tmock .l {
    background: #f6f6f7;
  }
  .tmock i {
    height: 8px;
    border-radius: 4px;
  }
  .tmock .d i {
    background: rgba(255, 255, 255, 0.18);
  }
  .tmock .l i {
    background: rgba(0, 0, 0, 0.14);
  }
  .tmock i:first-child {
    width: 60%;
  }
  .tmock .d i:first-child {
    background: #fff;
  }
  .tmock .l i:first-child {
    background: #111;
  }
  .tmock.dark .l,
  .tmock.light .d {
    display: none;
  }
  .vh {
    padding: 0 4px 2px;
    color: var(--faint);
    font-size: 12px;
  }
  .slider {
    margin-top: 12px;
    cursor: default;
  }
  .slider input {
    width: 180px;
    padding: 0;
    border: 0;
    background: none;
    accent-color: var(--accent);
  }
  .mini-seg {
    display: flex;
    background: var(--high);
    border-radius: var(--r);
    padding: 2px;
  }
  .mini-seg button {
    height: 26px;
    padding: 0 12px;
    border-radius: var(--r-s);
    font-size: 12px;
    color: var(--dim);
  }
  .mini-seg button.on {
    background: var(--accent);
    color: var(--on-accent);
  }
  .swatches {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .sw {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    outline: 1px solid var(--line-strong);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    font-weight: 700;
    color: var(--dim);
    position: relative;
  }
  .sw.on {
    outline: 2px solid var(--text);
    outline-offset: 2px;
  }
  .sw.mono {
    background: linear-gradient(135deg, #fff 50%, #000 50%);
  }
  .sw.sys {
    background: var(--high);
  }
  .sw.pick {
    background: var(--high);
    cursor: pointer;
    overflow: hidden;
  }
  .sw.pick input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }
  .pct {
    width: 40px;
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--dim);
  }
  .radio {
    width: 18px;
    height: 18px;
    flex: none;
    border-radius: 50%;
    outline: 2px solid var(--ghost);
    outline-offset: -2px;
    transition: outline 0.12s;
  }
  .radio.on {
    outline: 6px solid var(--accent);
    outline-offset: -6px;
  }
  .card .btn.sm {
    height: 26px;
    padding: 0 10px;
    font-size: 12px;
  }
  .vl {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 4px 2px;
  }
  .vl b {
    flex: 1;
    font-weight: 600;
  }
  .tick {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
  }

  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 10px;
  }
  .card {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    border-radius: var(--r-l);
    background: var(--panel);
    outline: 1px solid var(--line);
    text-align: left;
    transition: background 0.12s, outline-color 0.12s;
  }
  .card:hover {
    background: var(--high);
  }
  .card.on {
    outline-color: var(--line-strong);
  }
  .ic {
    width: 38px;
    height: 38px;
    border-radius: var(--r);
    display: flex;
    align-items: center;
    justify-content: center;
    flex: none;
    color: var(--faint);
    transition: background 0.15s, color 0.15s;
  }
  .card.on .ic {
    color: var(--text);
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .txt b {
    font-weight: 500;
  }
  .txt small {
    color: var(--faint);
    font-size: 12px;
  }
  .caps {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 3px;
  }
  kbd {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    height: 22px;
    padding: 0 6px;
    border-radius: var(--r-xs);
    background: var(--high);
    outline: 1px solid var(--line-strong);
    box-shadow: 0 2px 0 var(--line-strong);
    font: 11px var(--mono);
    color: var(--text);
  }
  .txt kbd {
    height: 18px;
    min-width: 18px;
    font-size: 10px;
    box-shadow: none;
  }
  .or {
    margin: 0 3px;
    color: var(--ghost);
  }

  .keys {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 4px 20px;
  }
  .key {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    min-height: 40px;
    padding: 6px 12px;
    border-radius: var(--r);
  }
  .key:hover {
    background: var(--hover);
  }
  .key .muted {
    text-align: right;
    font-size: 13px;
  }

  .about {
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 20px;
    border-radius: var(--r-xl);
    background: var(--panel);
    outline: 1px solid var(--line);
  }
  .about b {
    font-size: 15px;
    font-weight: 600;
  }
  .about p {
    margin: 3px 0 0;
    font-size: 12px;
  }
  .about .btn {
    gap: 8px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 80px 0;
    color: var(--faint);
  }

  @media (max-width: 760px) {
    nav {
      display: none;
    }
    .views {
      grid-template-columns: 1fr;
    }
  }
</style>
