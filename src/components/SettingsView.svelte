<script lang="ts">
  import Icon from './Icon.svelte'
  import { app, settings, toast } from '../lib/state.svelte'
  import { api } from '../lib/api'
  import { getVersion } from '@tauri-apps/api/app'

  let autostart = $state(false)
  let version = $state('')
  let checking = $state(false)
  api.getAutostart().then((v) => (autostart = v))
  getVersion().then((v) => (version = v))

  type Key = { [K in keyof typeof settings]: (typeof settings)[K] extends boolean ? K : never }[keyof typeof settings]
  const groups: { title: string; items: { key: Key; label: string; hint?: string }[] }[] = [
    {
      title: 'Przeglądanie',
      items: [
        { key: 'showHidden', label: 'Pokazuj ukryte i systemowe pliki', hint: 'Ctrl+H' },
        { key: 'foldersFirst', label: 'Foldery przed plikami' },
        { key: 'singleClickOpen', label: 'Otwieraj jednym kliknięciem' },
        { key: 'rememberViewPerFolder', label: 'Zapamiętuj widok i sortowanie dla każdego folderu' },
        { key: 'autoGallery', label: 'Foldery ze zdjęciami i filmami otwieraj w galerii' },
      ],
    },
    {
      title: 'Podgląd',
      items: [
        { key: 'previewPane', label: 'Panel podglądu', hint: 'Ctrl+P' },
        { key: 'playVideoPreview', label: 'Odtwarzaj wideo w panelu podglądu (bez dźwięku)' },
        { key: 'pdfThumbs', label: 'Miniatury PDF, gdy Windows ich nie dostarcza', hint: 'Renderowane w pamięci, nic nie jest zapisywane na dysku' },
      ],
    },
    {
      title: 'Bezpieczeństwo',
      items: [{ key: 'confirmTrash', label: 'Pytaj przed przeniesieniem do Kosza', hint: 'Usuwanie trwałe zawsze wymaga potwierdzenia' }],
    },
    {
      title: 'Działanie w tle',
      items: [
        { key: 'closeToTray', label: 'Zamknięcie okna chowa aplikację do zasobnika', hint: 'Tylko gdy są synchronizacje – dzięki temu działają dalej' },
        { key: 'checkUpdates', label: 'Sprawdzaj dostępność nowych wersji' },
      ],
    },
  ]

  const KEYS: [string, string][] = [
    ['Ctrl+T / Ctrl+W', 'Nowa / zamknij kartę'],
    ['Ctrl+Shift+T', 'Przywróć zamkniętą kartę'],
    ['Ctrl+Tab', 'Następna karta'],
    ['Ctrl+\\', 'Podział na dwa panele'],
    ['F6', 'Przełącz panel (w podziale)'],
    ['Ctrl+K', 'Szybkie przejście'],
    ['Ctrl+L', 'Edytuj ścieżkę'],
    ['Ctrl+F', 'Szukaj'],
    ['Alt+← / → / ↑', 'Wstecz / dalej / w górę'],
    ['Spacja', 'Podgląd pełnoekranowy'],
    ['F2', 'Zmień nazwę (Tab – następny)'],
    ['Del / Shift+Del', 'Kosz / trwale'],
    ['Ctrl+C / X / V', 'Kopiuj / wytnij / wklej (zgodne z Eksploratorem)'],
    ['Ctrl+Shift+C', 'Kopiuj ścieżkę'],
    ['Ctrl+Z', 'Cofnij'],
    ['Ctrl+Shift+N', 'Nowy folder'],
    ['Ctrl+1 / 2 / 3', 'Lista / siatka / galeria'],
    ['Ctrl+kółko', 'Wielkość miniatur'],
    ['Alt+Enter', 'Właściwości'],
    ['Ctrl+H', 'Ukryte pliki'],
  ]

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

<div class="settings">
  <h1>Ustawienia</h1>
  {#each groups as g (g.title)}
    <section>
      <h2 class="section-title">{g.title}</h2>
      <div class="opts">
        {#each g.items as it (it.key)}
          <button class="opt" onclick={() => (settings[it.key] = !settings[it.key])}>
            <span class="grow"><span>{it.label}</span>{#if it.hint}<small>{it.hint}</small>{/if}</span>
            <span class="switch" class:on={settings[it.key]}></span>
          </button>
        {/each}
        {#if g.title === 'Działanie w tle'}
          <button class="opt" onclick={toggleAutostart}>
            <span class="grow"><span>Uruchamiaj z systemem (w zasobniku)</span><small>Synchronizacja działa od razu po zalogowaniu</small></span>
            <span class="switch" class:on={autostart}></span>
          </button>
        {/if}
      </div>
    </section>
  {/each}

  <section>
    <h2 class="section-title">Domyślny widok</h2>
    <div class="row">
      <button class="chip" class:on={settings.defaultView === 'list'} onclick={() => (settings.defaultView = 'list')}>Lista</button>
      <button class="chip" class:on={settings.defaultView === 'grid'} onclick={() => (settings.defaultView = 'grid')}>Siatka</button>
      <button class="chip" class:on={settings.defaultView === 'gallery'} onclick={() => (settings.defaultView = 'gallery')}>Galeria</button>
    </div>
  </section>

  <section>
    <h2 class="section-title">Skróty klawiszowe</h2>
    <div class="keys">
      {#each KEYS as [k, l] (k)}
        <div><span class="kbd">{k}</span><span class="muted">{l}</span></div>
      {/each}
    </div>
  </section>

  <section>
    <h2 class="section-title">O aplikacji</h2>
    <div class="about">
      <div class="logo"><Icon name="folder" size={26} /></div>
      <div class="grow"><b>OpenFilePower dla Windows</b><p class="faint">Wersja {version} · miniatury i metadane pochodzą z systemu Windows</p></div>
      <button class="btn" onclick={check} disabled={checking}>{checking ? 'Sprawdzam…' : 'Sprawdź aktualizacje'}</button>
    </div>
  </section>
</div>

<style>
  .settings {
    flex: 1;
    overflow-y: auto;
    padding: 16px 24px 40px;
  }
  h1 {
    margin: 0 0 8px;
    font-size: 22px;
    font-weight: 600;
  }
  section {
    max-width: 760px;
    margin-top: 22px;
  }
  h2 {
    margin: 0 0 8px;
  }
  .opts {
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-radius: 14px;
    overflow: hidden;
  }
  .opt {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 12px 16px;
    background: var(--panel);
    text-align: left;
  }
  .opt:hover {
    background: var(--high);
  }
  .opt small {
    display: block;
    color: var(--faint);
    font-size: 12px;
    margin-top: 2px;
  }
  .keys {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px 24px;
  }
  .keys div {
    display: flex;
    gap: 12px;
    align-items: baseline;
  }
  .keys .kbd {
    width: 130px;
    flex: none;
    color: var(--text);
  }
  .about {
    display: flex;
    align-items: center;
    gap: 14px;
    background: var(--panel);
    border-radius: 14px;
    padding: 14px 16px;
  }
  .about p {
    margin: 2px 0 0;
    font-size: 12px;
  }
  .logo {
    width: 46px;
    height: 46px;
    border-radius: 14px;
    background: #fff;
    color: #000;
    display: flex;
    align-items: center;
    justify-content: center;
  }
</style>
