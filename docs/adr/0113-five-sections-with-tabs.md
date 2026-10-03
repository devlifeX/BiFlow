# 0113: Five sections with tabs, and site routing on Home

## Status

Accepted

## Context

The advanced UI had grown into long pages of stacked cards:

- The Dashboard led with five component cards and paragraphs.
- Clients lived on the Dashboard.
- List Management stacked lists, cloud rules, the pin table, and apps.
- Diagnostics stacked nine sections.
- About had its own sidebar entry.

The task people open the app for (connect, and send a site through VPN or
Direct) was not on the first screen.

## Decision

- Five sections: Home, Routing (Sites / Lists / Apps / Iran rules),
  Clients, Troubleshoot (Live / Test / Tools / Logs), and Settings
  (Network / Behavior / About). Page ids stay `dashboard`, `rules`,
  `diagnostics`, and `settings`; `clients` is new, and `about` maps to the
  Settings About tab, so the tray and stored navigation keep working.
- The store remembers the last tab per page.
- Settings About labels the credits as developers in both languages. List
  Dariush Vesal first, then Omis Asgari (امید عسگری), then Reza Mahdavi
  (رضا مهدوی) (6.2.59).
- Home shows:
  - the connection hero, with the default-route select;
  - an add-site bar that pins a pasted host to Direct or a client;
  - three facts, with one Health tile that holds the component rows;
  - the live diagram beside "Your sites" (the newest pins, editable in
    place).
- Basic mode shows the status, the lifecycle buttons, and the same
  add-site bar.
- Explanations move behind info tips, and confirmations become toasts.
- The mode switch moves to the bottom of the sidebar.

## Consequences

The everyday flow is one line on the first screen in both modes. Every
page is a title plus tabs. `docs/DESIGN.md` records the layout, and the
e2e specs navigate by section and tab (`goTo(page, section, tab)`).
