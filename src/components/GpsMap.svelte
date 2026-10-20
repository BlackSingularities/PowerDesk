<script lang="ts">
  import Icon from './Icon.svelte'
  import { api } from '../lib/api'

  let { lat, lon, height = 200 }: { lat: number; lon: number; height?: number } = $props()

  let zoom = $state(14)
  let width = $state(320)
  let failed = $state(false)

  // Rzut Web Mercator – współrzędne w pikselach całej mapy na danym przybliżeniu.
  const T = 256
  const world = (z: number) => T * 2 ** z
  const px = $derived((lon + 180) / 360 * world(zoom))
  const py = $derived(((1 - Math.log(Math.tan((lat * Math.PI) / 180) + 1 / Math.cos((lat * Math.PI) / 180)) / Math.PI) / 2) * world(zoom))

  const tiles = $derived.by(() => {
    const out: { x: number; y: number; left: number; top: number; key: string }[] = []
    const n = 2 ** zoom
    const x0 = Math.floor((px - width / 2) / T), x1 = Math.floor((px + width / 2) / T)
    const y0 = Math.floor((py - height / 2) / T), y1 = Math.floor((py + height / 2) / T)
    for (let x = x0; x <= x1; x++)
      for (let y = y0; y <= y1; y++) {
        if (y < 0 || y >= n) continue
        const wx = ((x % n) + n) % n
        out.push({ x: wx, y, left: x * T - px + width / 2, top: y * T - py + height / 2, key: `${zoom}/${x}/${y}` })
      }
    return out
  })

  const fmt = (v: number, pos: string, neg: string) => `${Math.abs(v).toFixed(5)}° ${v >= 0 ? pos : neg}`
  const osm = $derived(`https://www.openstreetmap.org/?mlat=${lat}&mlon=${lon}#map=${zoom}/${lat}/${lon}`)
  const gmaps = $derived(`https://www.google.com/maps/search/?api=1&query=${lat},${lon}`)
</script>

<div class="gps">
  <div class="map" bind:clientWidth={width} style:height="{height}px" role="img" aria-label="Mapa miejsca wykonania zdjęcia">
    {#if !failed}
      {#each tiles as t (t.key)}
        <img src="https://tile.openstreetmap.org/{zoom}/{t.x}/{t.y}.png" alt="" style:left="{t.left}px" style:top="{t.top}px" draggable="false" onerror={() => (failed = true)} />
      {/each}
    {:else}
      <div class="off"><Icon name="cloud" size={20} /><span>Mapa wymaga połączenia z internetem</span></div>
    {/if}
    <span class="pin" aria-hidden="true"></span>
    <div class="zoom">
      <button title="Przybliż" aria-label="Przybliż" disabled={zoom >= 18} onclick={() => zoom++}><Icon name="plus" size={14} /></button>
      <button title="Oddal" aria-label="Oddal" disabled={zoom <= 3} onclick={() => zoom--}><Icon name="min" size={14} /></button>
    </div>
    <span class="attr">© OpenStreetMap</span>
  </div>
  <div class="row coords">
    <span class="grow">{fmt(lat, 'N', 'S')}, {fmt(lon, 'E', 'W')}</span>
    <button class="lnk" onclick={() => { navigator.clipboard.writeText(`${lat}, ${lon}`) }} title="Kopiuj współrzędne"><Icon name="copy" size={13} /></button>
    <button class="lnk" onclick={() => api.open(gmaps)}>Google Maps</button>
    <button class="lnk" onclick={() => api.open(osm)}>OSM</button>
  </div>
</div>

<style>
  .gps {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .map {
    position: relative;
    overflow: hidden;
    border-radius: var(--r);
    background: var(--high);
    outline: 1px solid var(--line);
  }
  .map img {
    position: absolute;
    width: 256px;
    height: 256px;
    user-select: none;
  }
  :global([data-theme='dark']) .map img {
    filter: invert(1) hue-rotate(180deg) brightness(0.9) contrast(0.9);
  }
  .pin {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 16px;
    height: 16px;
    margin: -8px 0 0 -8px;
    border-radius: 50%;
    background: var(--rec);
    box-shadow: 0 0 0 3px #fff, 0 2px 8px rgba(0, 0, 0, 0.4);
  }
  .zoom {
    position: absolute;
    right: 8px;
    top: 8px;
    display: flex;
    flex-direction: column;
    border-radius: var(--r-s);
    overflow: hidden;
    background: var(--panel);
    box-shadow: 0 2px 8px var(--shadow);
  }
  .zoom button {
    width: 26px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--dim);
  }
  .zoom button:hover {
    background: var(--hover);
    color: var(--text);
  }
  .zoom button:disabled {
    opacity: 0.35;
  }
  .attr {
    position: absolute;
    right: 4px;
    bottom: 2px;
    font-size: 9px;
    color: var(--faint);
    background: var(--panel);
    padding: 0 4px;
    border-radius: 4px;
  }
  .off {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font-size: 12px;
    color: var(--faint);
  }
  .coords {
    font-size: 12px;
    color: var(--dim);
    gap: 4px;
  }
  .lnk {
    display: inline-flex;
    align-items: center;
    height: 22px;
    padding: 0 7px;
    border-radius: var(--r-xs);
    font-size: 11.5px;
    color: var(--dim);
  }
  .lnk:hover {
    background: var(--hover);
    color: var(--text);
  }
</style>
