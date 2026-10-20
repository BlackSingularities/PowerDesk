<script lang="ts">
  import Toolbar from './Toolbar.svelte'
  import FileView from './FileView.svelte'
  import Home from './Home.svelte'
  import SyncView from './SyncView.svelte'
  import SettingsView from './SettingsView.svelte'
  import StatusBar from './StatusBar.svelte'
  import { app, HOME, SYNC, SETTINGS, type Tab } from '../lib/state.svelte'

  let { tab, pane }: { tab: Tab; pane: number } = $props()
  let toolbar = $state<ReturnType<typeof Toolbar>>()
  const focused = $derived(app.focusedPane === pane)

  export function focusAddress() {
    toolbar?.focusAddress()
  }
  export function focusSearch() {
    toolbar?.focusSearch()
  }
</script>

<section class="pane" class:focused class:split={app.split} onmousedowncapture={() => (app.focusedPane = pane)} aria-label={tab.title}>
  <Toolbar bind:this={toolbar} {tab} {focused} />
  {#if tab.path === HOME}
    <Home {tab} />
  {:else if tab.path === SYNC}
    <SyncView {tab} />
  {:else if tab.path === SETTINGS}
    <SettingsView />
  {:else}
    <FileView {tab} {pane} />
  {/if}
  {#if tab.loading}<div class="loading"></div>{/if}
  <StatusBar {tab} />
</section>

<style>
  .pane {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border-radius: var(--r-l);
  }
  .pane.split {
    outline: 1px solid var(--line);
    outline-offset: -1px;
  }
  .pane.split.focused {
    outline-color: var(--line-strong);
  }
  .loading {
    position: absolute;
    left: 20%;
    right: 20%;
    top: 0;
    height: 2px;
    border-radius: 2px;
    background: linear-gradient(90deg, transparent, #fff, transparent);
    animation: slide 1s ease-in-out infinite;
  }
  @keyframes slide {
    0% {
      transform: translateX(-60%);
      opacity: 0;
    }
    50% {
      opacity: 1;
    }
    100% {
      transform: translateX(60%);
      opacity: 0;
    }
  }
</style>
