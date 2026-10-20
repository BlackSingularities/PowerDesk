<script lang="ts">
  import Icon from './Icon.svelte'
  import type { IconName } from '../lib/icons'
  import type { Prop } from '../lib/api'

  let { props, compact = false }: { props: Prop[]; compact?: boolean } = $props()

  // Metadane, które pokazujemy osobno (nazwa, rozmiar, daty pliku) albo które nic nie mówią.
  const SKIP = new Set([
    'System.ItemNameDisplay', 'System.ItemName', 'System.FileName', 'System.ParsingName', 'System.ItemPathDisplay', 'System.ItemFolderPathDisplay',
    'System.ItemFolderNameDisplay', 'System.ItemFolderPathDisplayNarrow', 'System.Size', 'System.ItemTypeText', 'System.ItemType', 'System.FileExtension',
    'System.DateModified', 'System.DateCreated', 'System.DateAccessed', 'System.FileAttributes', 'System.FileOwner', 'System.ComputerName',
    'System.ContentType', 'System.PerceivedType', 'System.Kind', 'System.KindText', 'System.SharingStatus', 'System.ZoneIdentifier', 'System.IsShared',
    'System.SFGAOFlags', 'System.Link.TargetParsingPath', 'System.ItemNameSortOverride', 'System.ThumbnailCacheId', 'System.StorageProviderUIStatus',
    'System.OfflineAvailability', 'System.OfflineStatus', 'System.Image.Dimensions', 'System.Video.FrameWidth', 'System.Video.FrameHeight',
    'System.Image.HorizontalSize', 'System.Image.VerticalSize', 'System.Photo.FNumber', 'System.Photo.ExposureTime', 'System.Photo.ISOSpeed',
    'System.Photo.FocalLength', 'System.Photo.ExposureBias', 'System.Photo.FocalLengthInFilm', 'System.Photo.Aperture', 'System.Photo.ShutterSpeed',
    'System.Photo.MaxAperture', 'System.Photo.ExposureTimeNumerator', 'System.Photo.ExposureTimeDenominator', 'System.Photo.FNumberNumerator',
    'System.Photo.FNumberDenominator', 'System.Photo.ApertureNumerator', 'System.Photo.ApertureDenominator',
  ])

  const SECTIONS: { id: string; title: string; icon: IconName; match: (k: string) => boolean }[] = [
    { id: 'photo', title: 'Aparat', icon: 'image', match: (k) => k.startsWith('System.Photo.') },
    { id: 'image', title: 'Obraz', icon: 'gallery', match: (k) => k.startsWith('System.Image.') },
    { id: 'gps', title: 'Lokalizacja', icon: 'pin', match: (k) => k.startsWith('System.GPS.') },
    { id: 'video', title: 'Wideo', icon: 'video', match: (k) => k.startsWith('System.Video.') },
    { id: 'audio', title: 'Dźwięk', icon: 'music', match: (k) => k.startsWith('System.Audio.') },
    { id: 'music', title: 'Muzyka', icon: 'music', match: (k) => k.startsWith('System.Music.') || k.startsWith('System.Media.') },
    { id: 'doc', title: 'Dokument', icon: 'doc', match: (k) => k.startsWith('System.Document.') || ['System.Title', 'System.Author', 'System.Company', 'System.Subject', 'System.Keywords', 'System.Comment', 'System.Category', 'System.ApplicationName'].includes(k) },
    { id: 'app', title: 'Program', icon: 'app', match: (k) => k.startsWith('System.Software.') || k === 'System.FileVersion' || k === 'System.FileDescription' || k === 'System.Copyright' },
    { id: 'other', title: 'Inne', icon: 'info', match: () => true },
  ]

  const byKey = $derived(new Map(props.map((p) => [p.key, p])))
  const num = (k: string) => byKey.get(k)?.num ?? null
  const val = (k: string) => byKey.get(k)?.value ?? ''

  const dims = $derived.by(() => {
    const w = num('System.Image.HorizontalSize') ?? num('System.Video.FrameWidth')
    const h = num('System.Image.VerticalSize') ?? num('System.Video.FrameHeight')
    return w && h ? { w, h, mp: (w * h) / 1e6 } : null
  })

  // ---------- Ekspozycja ----------
  const exp = $derived.by(() => {
    const f = num('System.Photo.FNumber')
    const t = num('System.Photo.ExposureTime')
    const iso = num('System.Photo.ISOSpeed')
    const fl = num('System.Photo.FocalLength')
    const fl35 = num('System.Photo.FocalLengthInFilm')
    const bias = num('System.Photo.ExposureBias')
    if (!f && !t && !iso && !fl) return null
    const ev = f && t ? Math.log2((f * f) / t) - Math.log2((iso || 100) / 100) : null
    return { f, t, iso, fl, fl35, bias, ev }
  })
  /** Położenie 0–1 na skali logarytmicznej. */
  const logPos = (v: number, lo: number, hi: number) => Math.max(0, Math.min(1, Math.log(v / lo) / Math.log(hi / lo)))
  const shutter = (t: number) => (t >= 1 ? `${+t.toFixed(1)}″` : `1/${Math.round(1 / t)}`)
  const scene = (ev: number) =>
    ev >= 14 ? 'pełne słońce' : ev >= 12 ? 'słonecznie, lekkie chmury' : ev >= 10 ? 'pochmurno' : ev >= 8 ? 'cień, zmierzch' : ev >= 5 ? 'jasne wnętrze' : ev >= 2 ? 'wnętrze, wieczór' : ev >= -2 ? 'noc w mieście' : 'głęboka noc'
  // Otwór przysłony: f/1.4 – prawie pełne koło, f/22 – mała szczelina.
  const iris = (f: number) => 5 + 21 * (1 - logPos(f, 1.2, 22))

  const sections = $derived.by(() => {
    const out = new Map<string, Prop[]>()
    for (const p of props) {
      if (SKIP.has(p.key) || !p.value) continue
      const s = SECTIONS.find((x) => x.match(p.key))!
      if (!out.has(s.id)) out.set(s.id, [])
      out.get(s.id)!.push(p)
    }
    return SECTIONS.filter((s) => out.has(s.id) || (s.id === 'photo' && exp)).map((s) => ({ ...s, items: out.get(s.id) ?? [] }))
  })
</script>

<div class="meta" class:compact>
  {#if dims}
    <div class="dims">
      <span class="big">{dims.w} × {dims.h}</span>
      <span class="faint">{dims.mp >= 0.1 ? `${dims.mp.toFixed(1)} Mpx` : 'px'}{val('System.Video.FrameRate') ? ` · ${(num('System.Video.FrameRate')! / 1000).toFixed(num('System.Video.FrameRate')! % 1000 ? 2 : 0)} kl./s` : ''}{val('System.Media.Duration') ? ` · ${val('System.Media.Duration')}` : ''}</span>
    </div>
  {/if}

  {#if exp}
    <div class="exposure">
      {#if exp.f}
        <div class="dial">
          <svg viewBox="0 0 64 64" class="iris" aria-hidden="true">
            <circle cx="32" cy="32" r="29" class="ring" />
            <circle cx="32" cy="32" r={iris(exp.f)} class="hole" />
            {#each Array(7) as _, i (i)}
              <line x1="32" y1="32" x2={32 + 29 * Math.cos((i * 2 * Math.PI) / 7)} y2={32 + 29 * Math.sin((i * 2 * Math.PI) / 7)} class="blade" />
            {/each}
          </svg>
          <b>f/{+exp.f.toFixed(1)}</b><span>przysłona</span>
        </div>
      {/if}
      {#if exp.t}
        <div class="dial">
          <div class="scale" title="od 30″ do 1/8000 s">
            <i style:left="{(1 - logPos(exp.t, 1 / 8000, 30)) * 100}%"></i>
          </div>
          <b>{shutter(exp.t)} s</b><span>czas naświetlania</span>
        </div>
      {/if}
      {#if exp.iso}
        <div class="dial">
          <div class="scale iso" title="ISO 50 – 25600">
            <i style:left="{logPos(exp.iso, 50, 25600) * 100}%"></i>
          </div>
          <b>ISO {exp.iso}</b><span>czułość</span>
        </div>
      {/if}
      {#if exp.fl}
        <div class="dial">
          <div class="scale" title="10 – 600 mm">
            <i style:left="{logPos(exp.fl35 || exp.fl, 10, 600) * 100}%"></i>
          </div>
          <b>{+exp.fl.toFixed(1)} mm</b><span>{exp.fl35 && exp.fl35 !== exp.fl ? `ogniskowa · ${exp.fl35} mm ekw.` : 'ogniskowa'}</span>
        </div>
      {/if}
      {#if exp.ev !== null}
        <div class="evrow">
          <span class="faint">Światło w scenie</span>
          <b>EV {exp.ev.toFixed(1)}</b>
          <span class="faint">· {scene(exp.ev)}</span>
          {#if exp.bias}<span class="bias">{exp.bias > 0 ? '+' : ''}{+exp.bias.toFixed(1)} EV korekty</span>{/if}
        </div>
      {/if}
    </div>
  {/if}

  {#each sections as s (s.id)}
    {#if s.items.length}
      <section>
        <h3><Icon name={s.icon} size={13} />{s.title}</h3>
        <dl>
          {#each s.items as p (p.key + p.name)}
            <dt>{p.name}</dt>
            <dd>
              {#if p.key === 'System.Rating' && p.num !== null}
                <span class="stars" title={p.value}>{#each Array(5) as _, i (i)}<span class:on={i < Math.round((p.num / 99) * 5)}>★</span>{/each}</span>
              {:else}{p.value}{/if}
            </dd>
          {/each}
        </dl>
      </section>
    {/if}
  {/each}
</div>

<style>
  .meta {
    display: flex;
    flex-direction: column;
    gap: 14px;
    user-select: text;
  }
  .dims {
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
  }
  .big {
    font-size: 18px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .exposure {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
    padding: 12px;
    border-radius: var(--r);
    background: var(--card);
  }
  .compact .exposure {
    grid-template-columns: repeat(2, 1fr);
  }
  .dial {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    text-align: center;
    min-width: 0;
  }
  .dial b {
    font-size: 14px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .dial span {
    font-size: 11px;
    color: var(--faint);
  }
  .iris {
    width: 44px;
    height: 44px;
  }
  .ring {
    fill: #222;
    stroke: var(--line-strong);
  }
  .blade {
    stroke: #333;
    stroke-width: 1;
  }
  .hole {
    fill: #fff;
    transition: r 0.3s var(--ease);
  }
  .scale {
    position: relative;
    width: 100%;
    height: 44px;
    display: flex;
    align-items: center;
  }
  .scale::before {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    height: 4px;
    border-radius: 4px;
    background: linear-gradient(90deg, var(--ghost), var(--dim));
  }
  .scale.iso::before {
    background: linear-gradient(90deg, var(--ok), var(--warn) 65%, var(--rec));
  }
  .scale i {
    position: absolute;
    width: 12px;
    height: 12px;
    margin-left: -6px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 0 0 3px rgba(0, 0, 0, 0.6);
  }
  .evrow {
    grid-column: 1 / -1;
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 6px;
    padding-top: 8px;
    margin-top: 2px;
    border-top: 1px solid var(--line);
    font-size: 12px;
  }
  .bias {
    margin-left: auto;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--high);
    color: var(--dim);
  }
  section h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 6px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--faint);
  }
  dl {
    display: grid;
    grid-template-columns: minmax(110px, 38%) 1fr;
    gap: 6px 14px;
    margin: 0;
    font-size: 12.5px;
  }
  dt {
    color: var(--faint);
  }
  dd {
    margin: 0;
    color: var(--dim);
    word-break: break-word;
  }
  .stars span {
    color: var(--ghost);
  }
  .stars span.on {
    color: var(--warn);
  }
</style>
