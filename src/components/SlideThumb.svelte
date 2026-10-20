<script lang="ts" module>
  import type { Deck } from '../lib/pptx'
  // Pierwsze slajdy w pamięci – przewijanie nie wczytuje plików ponownie.
  const cache = new Map<string, Promise<Deck | null>>()
</script>

<script lang="ts">
  import { fileUrl } from '../lib/api'

  let { path, modified, onfail }: { path: string; modified: number; onfail: () => void } = $props()
  let deck = $state<Deck | null>(null)

  $effect(() => {
    const key = `${path}|${modified}`
    if (!cache.has(key)) {
      cache.set(
        key,
        (async () => {
          const r = await fetch(fileUrl(path))
          if (!r.ok) return null
          const b = await r.arrayBuffer()
          if (b.byteLength > 150 * 2 ** 20) return null
          const { readDeck } = await import('../lib/pptx')
          return readDeck(b, 1)
        })().catch(() => null),
      )
    }
    cache.get(key)!.then((d) => (d?.slides.length ? (deck = d) : onfail()))
  })
</script>

{#if deck}
  {@const s = deck.slides[0]}
  <div class="wrap">
    <div class="slide" style:aspect-ratio="{deck.w} / {deck.h}" style:background={s.bg ?? '#fff'}>
      {#each s.items as it, i (i)}
        {#if it.kind === 'image'}
          <img src={it.src} alt="" style:left="{(it.x / deck.w) * 100}%" style:top="{(it.y / deck.h) * 100}%" style:width="{(it.w / deck.w) * 100}%" style:height="{(it.h / deck.h) * 100}%" />
        {/if}
      {/each}
      {#each s.items as it, i (i)}
        {#if it.kind === 'text'}
          <div class="tb" style:left="{(it.x / deck.w) * 100}%" style:top="{(it.y / deck.h) * 100}%" style:width="{(it.w / deck.w) * 100}%" style:height="{(it.h / deck.h) * 100}%">
            {#each it.paras.slice(0, 6) as p, k (k)}
              <p style:font-size="{(p.size / (deck.w / 12700)) * 100}cqw" style:font-weight={p.bold ? 700 : 400} style:color={p.color ?? '#222'} style:text-align={p.align === 'ctr' ? 'center' : p.align === 'r' ? 'right' : 'left'}>{p.text}</p>
            {/each}
          </div>
        {/if}
      {/each}
    </div>
  </div>
{/if}

<style>
  .wrap {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    container-type: inline-size;
  }
  .slide {
    position: relative;
    width: 100%;
    overflow: hidden;
    border-radius: 3px;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.35);
    font-family: Calibri, 'Segoe UI', sans-serif;
    container-type: inline-size;
  }
  img,
  .tb {
    position: absolute;
  }
  img {
    object-fit: contain;
  }
  .tb {
    overflow: hidden;
  }
  p {
    margin: 0;
    line-height: 1.15;
    white-space: pre-wrap;
  }
</style>
