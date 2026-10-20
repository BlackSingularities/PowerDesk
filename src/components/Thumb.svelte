<script lang="ts">
  import { thumbUrl, type Entry } from '../lib/api'
  import { settings } from '../lib/state.svelte'
  import { kindOf } from '../lib/util'
  import { pdfThumb } from '../lib/pdfthumb'

  let {
    entry,
    px,
    fit = 'contain',
    iconOnly = false,
    stamp = 0,
    pad = true,
  }: { entry: Entry; px: number; fit?: 'contain' | 'cover'; iconOnly?: boolean; stamp?: number; pad?: boolean } = $props()

  // Etapy: 0 – miniatura systemowa, 1 – PDF w pamięci, 2 – ikona systemowa.
  let stage = $state(0)
  let pdfSrc = $state<string | null>(null)

  const k = $derived(kindOf(entry))
  const wantsThumb = $derived(!iconOnly && !entry.dir && !entry.cloud && k !== 'app' && k !== 'audio' && k !== 'archive' && k !== 'code' && k !== 'text')

  $effect(() => {
    entry.path
    entry.modified
    stamp
    stage = wantsThumb ? 0 : 2
    pdfSrc = null
  })

  const src = $derived(
    stage === 0
      ? thumbUrl(entry.path, px, 't') + `&m=${entry.modified}`
      : stage === 1
        ? pdfSrc
        : thumbUrl(entry.path, Math.min(px, 256), 'i'),
  )

  async function fail() {
    if (stage === 0 && entry.ext === 'pdf' && settings.pdfThumbs) {
      stage = 1
      const url = await pdfThumb(entry.path, px, entry.modified)
      if (url) pdfSrc = url
      else stage = 2
    } else if (stage < 2) stage = 2
  }
</script>

{#if src}
  {#key src}
  <img
    {src}
    alt=""
    draggable="false"
    loading="lazy"
    decoding="async"
    class:icon={stage === 2 && pad}
    style:object-fit={stage === 2 ? 'contain' : fit}
    onerror={fail}
  />
  {/key}
{/if}

<style>
  img {
    width: 100%;
    height: 100%;
    display: block;
    animation: appear 0.18s ease-out;
  }
  @keyframes appear {
    from {
      opacity: 0;
    }
  }
  img.icon {
    padding: 14%;
    image-rendering: auto;
  }
</style>
