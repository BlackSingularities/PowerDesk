import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export interface Entry {
  name: string
  path: string
  dir: boolean
  size: number
  modified: number
  created: number
  ext: string
  hidden: boolean
  system: boolean
  readonly: boolean
  link: boolean
  cloud: boolean
}

export interface Drive {
  path: string
  letter: string
  label: string
  kind: 'fixed' | 'removable' | 'network' | 'cdrom' | 'ram' | 'unknown'
  fs: string
  total: number
  free: number
  serial: number
  remote: string | null
  online: boolean
}

export interface Known { id: string; name: string; path: string }
export interface Prop { name: string; value: string; key: string; num: number | null }
export interface MediaInfo { w: number; h: number; dur: number; taken: number }
export interface TextPreview { text: string; truncated: boolean; kind: string }
export interface ZipItem { name: string; size: number; packed: number; dir: boolean }
export interface DirSize { size: number; files: number; dirs: number }

export type ConflictPolicy = 'replace' | 'skip' | 'rename' | 'newer'
export interface ConflictInfo {
  name: string; src: string; dst: string
  src_size: number; dst_size: number; src_modified: number; dst_modified: number; dir: boolean
}

export interface OpProgress {
  id: number; kind: string; title: string
  done_bytes: number; total_bytes: number; done_items: number; total_items: number
  current: string; speed: number; state: 'running' | 'done' | 'error' | 'cancelled'
  errors: string[]; dest: string
}

export type SyncMode = 'mirror' | 'twoway' | 'backup'
export interface SyncTarget { path: string; rel: string; serial: number; label: string; network: boolean }
export interface SyncProfile {
  id: string; name: string; source: string; target: SyncTarget; mode: SyncMode
  enabled: boolean; continuous: boolean; interval_min: number; versioning: boolean
  exclude: string[]; include_hidden: boolean
}
export interface SyncCounts {
  to_target: number; to_source: number; del_target: number; del_source: number
  conflicts: number; bytes: number; errors: number
}
export interface SyncStatus {
  id: string; connected: boolean; resolved_target: string
  last_seen: number; last_sync: number; last_result: string; last_counts: SyncCounts
  state: '' | 'idle' | 'scanning' | 'syncing' | 'waiting' | 'offline' | 'paused' | 'confirm' | 'error'
  pending: number; progress: number; current: string
  needs_confirm: SyncCounts | null
  history: { at: number; summary: string; ok: boolean }[]
}
export interface SearchBatch { id: number; items: Entry[]; done: boolean; scanned: number }
export interface UpdateInfo { current: string; latest: string; newer: boolean; url: string; notes: string }

export const api = {
  listDir: (path: string) => invoke<Entry[]>('list_dir', { path }),
  stat: (path: string) => invoke<Entry>('stat', { path }),
  drives: () => invoke<Drive[]>('list_drives'),
  known: () => invoke<Known[]>('known_folders'),
  open: (path: string) => invoke<void>('open_path', { path }),
  openWith: (path: string) => invoke<boolean>('open_with', { path }),
  systemProps: (path: string) => invoke<boolean>('show_properties', { path }),
  nativeMenu: (paths: string[], x: number, y: number, extended = false) => invoke<boolean>('native_menu', { paths, x, y, extended }),
  reveal: (path: string) => invoke<void>('reveal', { path }),
  terminal: (path: string) => invoke<void>('open_terminal', { path }),
  mkdir: (parent: string, name: string) => invoke<string>('create_folder', { parent, name }),
  mkfile: (parent: string, name: string) => invoke<string>('create_file', { parent, name }),
  rename: (path: string, name: string) => invoke<string>('rename', { path, name }),
  extractItems: (sources: string[], dest: string) => invoke<string[]>('extract_items', { sources, dest }),
  createZip: (sources: string[], dest: string) => invoke<string>('create_zip', { sources, dest }),
  extractHere: (path: string) => invoke<string>('extract_here', { path }),
  openArchived: (path: string) => invoke<string>('open_archived', { path }),
  ftpLogin: (host: string, port: number, user: string, pass: string, remember: boolean, path = '') => invoke<string>('ftp_login', { host, port, user, pass, remember, path }),
  ftpForget: (path: string) => invoke<void>('ftp_forget', { path }),
  remoteDownload: (sources: [string, boolean][], dest: string) => invoke<number>('remote_download', { sources, dest }),
  remoteUpload: (sources: string[], dest: string) => invoke<number>('remote_upload', { sources, dest }),
  remoteDelete: (items: [string, boolean][]) => invoke<number>('remote_delete', { items }),
  remoteMkdir: (parent: string, name: string) => invoke<string>('remote_mkdir', { parent, name }),
  remoteRename: (path: string, name: string) => invoke<string>('remote_rename', { path, name }),
  remoteOpen: (path: string) => invoke<string>('remote_open', { path }),
  sshfsAvailable: () => invoke<boolean>('sshfs_available'),
  integration: () => invoke<{ installed: boolean; default_fm: boolean; context_menu: boolean; exe: string }>('integration'),
  setDefaultFm: (on: boolean) => invoke<void>('set_default_fm', { on }),
  setContextMenu: (on: boolean) => invoke<void>('set_context_menu', { on }),
  downloadUpdate: () => invoke<string>('download_update'),
  installUpdate: (path: string) => invoke<void>('install_update', { path }),
  systemAccent: () => invoke<string | null>('system_accent'),
  recentFiles: (dirs: string[], days: number, limit: number) => invoke<Entry[]>('recent_files', { dirs, days, limit }),
  renameMany: (pairs: [string, string][]) => invoke<{ done: number; errors: string[] }>('rename_many', { pairs }),
  startOp: (kind: 'copy' | 'move' | 'trash' | 'delete', sources: string[], dest = '', conflict: ConflictPolicy = 'rename') =>
    invoke<number>('start_op', { kind, sources, dest, conflict }),
  cancelOp: (id: number) => invoke<void>('cancel_op', { id }),
  conflicts: (sources: string[], dest: string) => invoke<ConflictInfo[]>('check_conflicts', { sources, dest }),
  undo: () => invoke<string>('undo'),
  redo: () => invoke<string>('redo'),
  search: (root: string, query: string, hidden: boolean) => invoke<number>('search', { root, query, hidden }),
  cancelSearch: () => invoke<void>('cancel_search'),
  dirSize: (path: string) => invoke<DirSize>('dir_size', { path }),
  hash: (path: string) => invoke<string>('file_hash', { path }),
  mediaInfo: (items: [string, number][]) => invoke<MediaInfo[]>('media_info', { items }),
  eject: (path: string) => invoke<void>('eject', { path }),
  diskCleanup: (letter: string) => invoke<void>('disk_cleanup', { letter }),
  shellProps: (path: string) => invoke<Prop[]>('shell_props', { path }),
  readText: (path: string, limit?: number) => invoke<TextPreview>('read_text', { path, limit }),
  officeText: (path: string) => invoke<TextPreview>('office_text', { path }),
  zipList: (path: string) => invoke<ZipItem[]>('zip_list', { path }),
  watch: (paths: string[]) => invoke<void>('watch_dirs', { paths }),
  clipSet: (paths: string[], cut: boolean) => invoke<void>('clipboard_set_files', { paths, cut }),
  clipGet: () => invoke<{ paths: string[]; cut: boolean }>('clipboard_get_files'),
  loadState: () => invoke<string>('load_state'),
  saveState: (json: string) => invoke<void>('save_state', { json }),
  closeToTray: (on: boolean) => invoke<void>('set_close_to_tray', { on }),
  startHidden: () => invoke<boolean>('start_hidden'),
  quit: () => invoke<void>('quit_app'),
  pickFolder: (start?: string) => invoke<string | null>('pick_folder', { start }),
  getAutostart: () => invoke<boolean>('get_autostart'),
  setAutostart: (on: boolean) => invoke<void>('set_autostart', { on }),
  checkUpdate: () => invoke<UpdateInfo>('check_update'),
  mapDrive: (letter: string | null, remote: string, persistent: boolean) => invoke<string>('map_drive', { letter, remote, persistent }),
  unmapDrive: (letter: string, forget: boolean) => invoke<void>('unmap_drive', { letter, forget }),
  syncList: () => invoke<{ profiles: SyncProfile[]; status: SyncStatus[] }>('sync_list'),
  syncSave: (profile: SyncProfile) => invoke<SyncProfile>('sync_save', { profile }),
  syncDelete: (id: string) => invoke<void>('sync_delete', { id }),
  syncNow: (id?: string, confirm = false) => invoke<void>('sync_now', { id: id ?? null, confirm }),
  syncEnable: (id: string, enabled: boolean) => invoke<void>('sync_enable', { id, enabled }),
  syncCancel: (id: string) => invoke<void>('sync_cancel', { id }),
  syncPreview: (id: string) => invoke<{ counts: SyncCounts; items: string[] }>('sync_preview', { id }),
}

export function on<T>(event: string, cb: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => cb(e.payload))
}

/** Plik bezpośrednio z dysku (obrazy, wideo, PDF) – strumieniowo, z obsługą przewijania wideo. */
export const fileUrl = (path: string) => convertFileSrc(path)

const BUCKETS = [32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1600, 2560]
export function bucket(px: number) {
  const want = px * (window.devicePixelRatio || 1)
  return BUCKETS.find((b) => b >= want) ?? 2560
}

/** Miniatura z systemowego cache Windows. mode 't' – tylko prawdziwa miniatura, 'i' – ikona systemowa. */
export function thumbUrl(path: string, px: number, mode: 't' | 'i' = 't') {
  return `http://thumb.localhost/${bucket(px)}/${mode}?p=${encodeURIComponent(path)}`
}
