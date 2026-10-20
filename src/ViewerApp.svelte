<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import Viewer from './components/Viewer.svelte'
  import Overlay from './components/Overlay.svelte'
  import { app, settings, VIEWER_KEY } from './lib/state.svelte'
  import { api, type Entry } from './lib/api'

  // Osobne okno podglądu: lista plików przychodzi z głównego okna przez localStorage (ten sam origin).
  function take() {
    try {
      const v = JSON.parse(localStorage.getItem(VIEWER_KEY) ?? 'null') as { list: Entry[]; index: number } | null
      if (v?.list?.length) app.viewer = { list: v.list, index: v.index }
    } catch {
      /* uszkodzony wpis – zostaje to, co było */
    }
  }
  take()
  window.addEventListener('storage', (e) => e.key === VIEWER_KEY && take())

  // Motyw i akcent jak w głównym oknie.
  api.loadState().then((json) => {
    try {
      const s = JSON.parse(json)
      if (s.settings) Object.assign(settings, s.settings)
    } catch {
      /* domyślne */
    }
    const light = settings.theme === 'light' || (settings.theme === 'system' && matchMedia('(prefers-color-scheme: light)').matches)
    document.documentElement.dataset.theme = light ? 'light' : 'dark'
    getCurrentWindow().show()
  })

  $effect(() => {
    const e = app.viewer?.list[app.viewer.index]
    if (e) getCurrentWindow().setTitle(`${e.name} – OpenFilePower`)
  })
</script>

{#if app.viewer}<Viewer standalone />{/if}
<Overlay />
