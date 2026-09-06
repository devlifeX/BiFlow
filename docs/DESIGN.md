# BiFlow UI design system

Rules the UI must keep. Every UI change is checked against this file; when a
rule has to change, update this file in the same commit.

## Density and rhythm

- Cards: `rounded-2xl border border-ink/10 bg-surface p-3.5` (inner tiles may
  use `p-3`). Page sections use `flex flex-col gap-3 pb-2`.
- No oversized hero paddings (`p-5`+) on regular cards; compact is the
  default everywhere.

## Cards with actions → footer pattern

- A card whose primary interaction is a button (Diagnostics tiles: Fresh
  Hiddify start, Permanent debug.log, Support bundle) puts every button in a
  **footer**: `mt-auto flex flex-wrap items-center gap-2 border-t
border-ink/10 pt-3`, with small buttons
  (`rounded-lg px-3 py-1.5 text-xs font-semibold`, icon `size={14}`).
- Tile siblings in one grid row must be equal height: the grid stretches
  children (`[&>div]:flex [&>div]:h-full [&>div]:flex-col` or per-card
  `flex h-full flex-col`) and `mt-auto` pins footers to the bottom, so
  footers align across the row.

## Section order and tiling

- Page sections are ordered by how often they are used: in Diagnostics the
  live-connections list comes first, then Test flow, then Reachability +
  Test timeline as a two-column pair, then the three utility tiles, then
  logs.
- Small independent utilities tile side-by-side (`grid gap-3 xl:grid-cols-3`)
  instead of stacking full-width.

## List Management

- Rule lists render as **full-width horizontal rows stacked vertically**
  (`flex flex-col gap-2`), one bar per list: name (inline-editable), entry
  count, Send-through select, Check, add-entry form, delete icon; entries as
  removable chips below the bar. Never a multi-column card grid here.

## Client cards (Dashboard)

- Compact by default: header (title, default badge, status pill, Enabled),
  one summary line (`N domains · M IPs · Local port P · Exit IP x.x.x.x`),
  optional warning banner (e.g. OpenVPN missing + platform download link).
- Everything else lives in a collapsible `<details>` labelled
  "Settings & pinned hosts".

## Colors

- Per-client accent colors come from `CLIENT_COLORS` / `clientColor()` in
  `apps/desktop/src/lib/outbound.ts` — the single source used by the live
  traffic diagram and the live-connections badges. DIRECT is always green.

## Live traffic diagram

- One branch per enabled client plus DIRECT; packets are rAF-driven along
  the measured path (SMIL is unreliable in the webview), labels alternate
  above/below the dot, births are staggered, and opacity fades in/out.
  Nodes are keyboard-accessible buttons with a status tooltip (client exit
  IP; DIRECT shows the real public IP).

## Chrome

- The settings-apply banner is `sticky top-0 z-40` inside the scroll
  container with `backdrop-blur` so it stays visible while scrolled.
- Scrollbars are themed globally (thin, `rgb(var(--ink) / 0.22)` thumb,
  transparent track); the OS default scrollbar must never appear.
- App boot renders `PageSkeleton` — a per-page structured skeleton, not a
  centered spinner.
- Pause/Disconnect (and Cancel) stay on one row on `sm+`
  (`sm:flex-nowrap`).
- Connection lifecycle buttons (`ConnectionActionButton`, cancel) use a fixed
  `128×30px` (`w-32 h-[30px]`) footprint, `whitespace-nowrap`, and a reserved
  tabular-nums countdown slot so labels never wrap and the control row does not
  shift while Connect/Pause/Resume/Disconnect progress runs.
- Component readiness uses one bordered list (`divide-y`, `h-8` rows) instead of
  per-component cards. Status chips are fixed `w-16 h-[18px]` with short labels
  (`Idle`, `Starting`, `Ready`, `Failed`, `Off`).
- Stat strip uses `flex-[2]` for Exit IP and `flex-1` for Providers and Active
  clients. Sidebar width is `w-44` with `h-7` nav rows.

## Accessibility guardrails

- Never use `sr-only` labels for inputs inside scrollable pages (they anchor
  to the page and break the no-document-overflow e2e); use `aria-label`.
- Interactive SVG nodes get `role="button"`, `tabIndex`, and Enter/Space
  handling.
- Both languages must pass the responsive e2e (no horizontal overflow at
  390px; tables scroll inside their own container).
