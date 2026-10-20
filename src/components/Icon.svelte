<script lang="ts">
  import { I, type IconName } from '../lib/icons'
  let { name, size = 18, fill = false }: { name: IconName; size?: number; fill?: boolean } = $props()
</script>

<svg class="ico" viewBox="0 0 24 24" width={size} height={size} style:fill={fill ? 'currentColor' : undefined} aria-hidden="true">
  <path d={I[name]} />
</svg>
