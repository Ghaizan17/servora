/** Inline SVG icons (stroke based, 1.6px, currentColor). */

function svg(paths, size = 16) {
  return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;
}

export const icons = {
  dashboard: (size) =>
    svg(
      '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',
      size,
    ),
  logs: (size) =>
    svg(
      '<path d="M4 5h16M4 10h16M4 15h10M4 20h13"/>',
      size,
    ),
  settings: (size) =>
    svg(
      '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1 1.55V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1.11-1.55 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-1.55-1H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.64 8.9a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34H9a1.7 1.7 0 0 0 1-1.55V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1 1.55 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87V9a1.7 1.7 0 0 0 1.55 1H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.51 1z"/>',
      size,
    ),
  server: (size) =>
    svg(
      '<rect x="3" y="4" width="18" height="6" rx="1.6"/><rect x="3" y="14" width="18" height="6" rx="1.6"/><path d="M7 7h.01M7 17h.01"/>',
      size,
    ),
  power: (size) =>
    svg('<path d="M12 4v8"/><path d="M6.6 7.4a7 7 0 1 0 10.8 0"/>', size),
  restart: (size) =>
    svg(
      '<path d="M20 11a8 8 0 1 0-2.6 5.9"/><path d="M20 5v6h-6"/>',
      size,
    ),
  refresh: (size) =>
    svg('<path d="M21 12a9 9 0 1 1-3-6.7"/><path d="M21 4v5h-5"/>', size),
  copy: (size) =>
    svg(
      '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M15 5.5A2.5 2.5 0 0 0 12.5 3H6a2 2 0 0 0-2 2v7.5A2.5 2.5 0 0 0 6.5 15"/>',
      size,
    ),
  check: (size) => svg('<path d="M20 6 9 17l-5-5"/>', size),
  alert: (size) =>
    svg(
      '<path d="M12 9v4"/><path d="M12 17h.01"/><circle cx="12" cy="12" r="9"/>',
      size,
    ),
  play: (size) => svg('<path d="M7 4l12 8-12 8z"/>', size),
  stop: (size) => svg('<rect x="6" y="6" width="12" height="12" rx="2"/>', size),
  shield: (size) =>
    svg('<path d="M12 3l7 3v6c0 4.2-2.9 7.9-7 9-4.1-1.1-7-4.8-7-9V6z"/>', size),
  info: (size) =>
    svg('<circle cx="12" cy="12" r="9"/><path d="M12 11v5"/><path d="M12 8h.01"/>', size),
  globe: (size) =>
    svg(
      '<circle cx="12" cy="12" r="9"/><path d="M3 12h18"/><path d="M12 3a15 15 0 0 1 0 18a15 15 0 0 1 0-18"/>',
      size,
    ),
  folder: (size) =>
    svg('<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>', size),
  plus: (size) => svg('<path d="M12 5v14"/><path d="M5 12h14"/>', size),
  search: (size) =>
    svg('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>', size),
  pencil: (size) =>
    svg('<path d="M4 20h4l10-10a2.5 2.5 0 0 0-3.5-3.5L4 16z"/><path d="m14.5 6.5 3 3"/>', size),
  trash: (size) =>
    svg(
      '<path d="M4 7h16"/><path d="M9 7V5h6v2"/><path d="M6 7l1 13h10l1-13"/><path d="M10 11v5"/><path d="M14 11v5"/>',
      size,
    ),
  external: (size) =>
    svg('<path d="M14 4h6v6"/><path d="M20 4 10 14"/><path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4"/>', size),
  stethoscope: (size) =>
    svg(
      '<path d="M6 3v6a4 4 0 0 0 8 0V3"/><path d="M10 13v3a5 5 0 0 0 10 0v-3"/><circle cx="20" cy="7" r="2"/>',
      size,
    ),
  download: (size) =>
    svg('<path d="M12 4v10"/><path d="m8 11 4 4 4-4"/><path d="M5 19h14"/>', size),
  x: (size) => svg('<path d="M6 6l12 12"/><path d="M18 6 6 18"/>', size),
  chevron: (size) => svg('<path d="m6 9 6 6 6-6"/>', size),
};
