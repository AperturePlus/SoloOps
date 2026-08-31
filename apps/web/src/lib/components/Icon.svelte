<script lang="ts">
  // Minimal inline icon set (stroke-based, 24 viewBox). Usage:
  //   <Icon name="box" size={16} />
  export type IconName =
    | "activity"
    | "alert"
    | "arrow-left"
    | "arrow-right"
    | "arrow-up-right"
    | "box"
    | "check"
    | "chevron-down"
    | "chevron-right"
    | "circle"
    | "clock"
    | "command"
    | "copy"
    | "database"
    | "eye"
    | "file"
    | "folder"
    | "gear"
    | "hash"
    | "info"
    | "layers"
    | "layout"
    | "list"
    | "lock"
    | "logout"
    | "mail"
    | "more-horizontal"
    | "pencil"
    | "play"
    | "plus"
    | "refresh"
    | "search"
    | "server"
    | "shield"
    | "spark"
    | "terminal"
    | "trash"
    | "triangle"
    | "user"
    | "x";

  let {
    name,
    size = 16,
    class: className = ""
  }: { name: IconName; size?: number; class?: string } = $props();

  const PATHS: Record<IconName, string> = {
    activity: '<path d="M3 12h3l2.5-7 4 14 2.5-7h3" />',
    alert:
      '<path d="m21.7 18-8-14a2 2 0 0 0-3.5 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.7-3Z" /><path d="M12 9v4" /><path d="M12 17h.01" />',
    "arrow-left": '<path d="M19 12H5" /><path d="m12 19-7-7 7-7" />',
    "arrow-right": '<path d="M5 12h14" /><path d="m12 5 7 7-7 7" />',
    "arrow-up-right": '<path d="M7 17 17 7" /><path d="M8 7h9v9" />',
    box: '<path d="M21 8.4v7.2a2 2 0 0 1-1 1.73l-7 4.04a2 2 0 0 1-2 0l-7-4.04a2 2 0 0 1-1-1.73V8.4a2 2 0 0 1 1-1.73l7-4.04a2 2 0 0 1 2 0l7 4.04a2 2 0 0 1 1 1.73Z" /><path d="m3.3 7 8.7 5 8.7-5" /><path d="M12 22V12" />',
    check: '<path d="M20 6 9 17l-5-5" />',
    "chevron-down": '<path d="m6 9 6 6 6-6" />',
    "chevron-right": '<path d="m9 6 6 6-6 6" />',
    circle: '<circle cx="12" cy="12" r="9" />',
    clock: '<circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" />',
    command: '<path d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3" />',
    copy: '<rect x="9" y="9" width="12" height="12" rx="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />',
    database:
      '<ellipse cx="12" cy="5" rx="8" ry="3" /><path d="M4 5v14c0 1.66 3.58 3 8 3s8-1.34 8-3V5" /><path d="M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3" />',
    eye: '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z" /><circle cx="12" cy="12" r="3" />',
    file: '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z" /><path d="M14 2v6h6" />',
    folder:
      '<path d="M3 6a2 2 0 0 1 2-2h4l2 3h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />',
    gear: '<circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.01a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h.01a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.01a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" />',
    hash: '<path d="M4 9h16" /><path d="M4 15h16" /><path d="M10 3 8 21" /><path d="M16 3l-2 18" />',
    info: '<circle cx="12" cy="12" r="9" /><path d="M12 8h.01" /><path d="M12 11v5" />',
    layers:
      '<path d="m12 2 8.5 5-8.5 5-8.5-5Z" /><path d="m4 12 8 4.7 8-4.7" /><path d="m4 17 8 4.7 8-4.7" />',
    layout:
      '<rect x="3" y="3" width="18" height="18" rx="2" /><path d="M3 9h18" /><path d="M9 21V9" />',
    list: '<path d="M8 6h13" /><path d="M8 12h13" /><path d="M8 18h13" /><path d="M3 6h.01" /><path d="M3 12h.01" /><path d="M3 18h.01" />',
    lock: '<rect x="4" y="11" width="16" height="10" rx="2" /><path d="M8 11V7a4 4 0 0 1 8 0v4" />',
    logout:
      '<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" /><path d="m16 17 5-5-5-5" /><path d="M21 12H9" />',
    mail: '<rect x="2" y="4" width="20" height="16" rx="2" /><path d="m22 7-10 6L2 7" />',
    "more-horizontal":
      '<circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /><circle cx="5" cy="12" r="1" />',
    pencil: '<path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />',
    play: '<polygon points="6 3 20 12 6 21 6 3" />',
    plus: '<path d="M12 5v14" /><path d="M5 12h14" />',
    refresh:
      '<path d="M3 12a9 9 0 0 1 15-6.7L21 8" /><path d="M21 3v5h-5" /><path d="M21 12a9 9 0 0 1-15 6.7L3 16" /><path d="M3 21v-5h5" />',
    search: '<circle cx="11" cy="11" r="7" /><path d="m21 21-4.3-4.3" />',
    server:
      '<rect x="2" y="3" width="20" height="7" rx="2" /><rect x="2" y="14" width="20" height="7" rx="2" /><path d="M6 6.5h.01" /><path d="M6 17.5h.01" />',
    shield: '<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z" />',
    spark:
      '<path d="M12 3v3" /><path d="M12 18v3" /><path d="M3 12h3" /><path d="M18 12h3" /><path d="m5.6 5.6 2.1 2.1" /><path d="m16.3 16.3 2.1 2.1" /><path d="m18.4 5.6-2.1 2.1" /><path d="m7.7 16.3-2.1 2.1" />',
    terminal: '<path d="m4 17 6-6-6-6" /><path d="M12 19h8" />',
    trash:
      '<path d="M3 6h18" /><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" /><path d="M10 11v6" /><path d="M14 11v6" />',
    triangle: '<path d="M21 21H3L12 3Z" />',
    user: '<circle cx="12" cy="8" r="4" /><path d="M4 21c0-4 3.6-6.5 8-6.5s8 2.5 8 6.5" />',
    x: '<path d="M18 6 6 18" /><path d="m6 6 12 12" />'
  };
</script>

<svg
  viewBox="0 0 24 24"
  width={size}
  height={size}
  fill="none"
  stroke="currentColor"
  stroke-width="1.7"
  stroke-linecap="round"
  stroke-linejoin="round"
  class={className}
  aria-hidden="true"
>
  <!-- PATHS is a static whitelist of hand-written icons; no user input reaches it. -->
  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
  {@html PATHS[name]}
</svg>
