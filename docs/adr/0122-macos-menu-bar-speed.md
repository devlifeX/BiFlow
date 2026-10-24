# ADR 0122: Physical adapter speeds in the macOS menu bar

Status: Accepted (6.2.72)

macOS already has a status item for BiFlow. Reuse that tray, menu, and
lifecycle rather than adding a second status item. Draw upload above download
with the arrow on the right of each rate, in an 11-column monospaced title, so
the item keeps one width while the digits change. Eleven columns fit the
longest automatic rate (`99.9 Kbps`). A longer fixed unit widens both lines
together. Hide the status icon while the speeds are shown, and put it back
when the title is cleared. The shared formatter and the saved
`show_taskbar_speed` / `taskbar_speed_unit` preferences supply the rates.
Settings → Speed display is available on macOS and labels the toggle “Show
network speed in menu bar”. It is off by default. Disabling clears only the
title and keeps the menu.

`TrayIcon::set_title` draws one proportional line and does not clear a title
when passed `None`. The menu bar path therefore sets an attributed title on
the existing status button: 9-point monospaced type, `labelColor`, and a
9-point line height so both lines fit the menu bar. The button is the
`TaoTrayTarget` superview. After each change the click overlay is resized to
the button. If that button cannot be found, the same text is submitted with
`set_title` and the snapshot records `plain_title`. Ubuntu keeps the
single-line `AppIndicator` label.

Read counters from `IFMIB_IFDATA` / `IFDATA_GENERAL`. `NET_RT_IFLIST2`'s
`if_data64.ifi_ibytes` does not match the 64-bit link total `netstat -ib`
reports on current Darwin, so that source must not drive the label. Wi-Fi
interfaces report `IFT_ETHER`; classify them with `SIOCGIFMEDIA`
(`IFM_IEEE80211` versus `IFM_ETHER`) and treat a link as up only when
`IFM_ACTIVE` is set. Exclude virtual prefixes (`utun`, `awdl`, `bridge`,
`anpi`, and the other tunnel and private interfaces) even when their flags
look up. Choose the active hardware adapter that owns the global default
route. No names, addresses, or routes leave the provider.

The one-second sampler, unavailable dashes, and preference application match
Ubuntu. Sampling continues while disconnected and while the main window is
hidden, and it stops on exit. Environment snapshots record `menu_bar_label`,
the adapter class, and whether the title was submitted. `label_set` proves
BiFlow's status button was found and the stacked title was applied.

Coverage includes virtual-name and media classification, default-route
selection, a host read without elevation, equal-width menu bar lines, macOS
settings unit tests, and the English/Persian speed-save flow.

Amendment (6.2.77): the one-second sampler used to call `sizeToFit`, move the
click overlay, and clear the status image on every tick, including while the
speeds were hidden. On macOS 26 that enters `NSStatusItemScene updateSettings`,
which waits with `performAsyncAndWait`. A hang on 23 October 2026 left the
main thread there for 73 minutes (`blocked by queue in a transient state`).
A tray-menu rebuild on a worker called `MenuItem::with_id`, which waits for
that same thread, so the menu never returned and the window had to be force
quit. Digit changes that keep the fixed column width now replace only the
attributed title. `sizeToFit`, the overlay frame, and the icon change only
when speeds are shown or hidden, when a line gets wider, or when the logo is
visible again. Identical text does not touch `AppKit`. Tray menu rebuilds run
on the main thread and only when the phase or busy flag changes.
