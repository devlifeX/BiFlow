# ADR 0074: Happ discovery and hidden Google hosts

## Status

Accepted

## Context

Connect logged `happ is not running and its executable was not found` even
when the Debian package was installed. The GUI image is `/opt/happ/bin/Happ`;
PATH only exposes the lowercase symlink `/usr/bin/happ`. Discovery compared
the bypass token `Happ` exactly, so Auto never launched it and `google.com`
pins to that client stayed dark.

`google.com` is also a fixed Diagnostics probe (ADR 0066) and a common Happ
pin. Shipping that hostname in the production UI is undesirable; debug and
`./dev.sh` still need the probe.

## Decision

- Resolve Happ (and other `LocalProxy` presets) with a case-insensitive
  filename match on PATH, plus well-known install paths
  (`/usr/bin/happ`, `/opt/happ/bin/Happ`, `%LOCALAPPDATA%\Happ\Happ.exe`).
- Probe `google.com` through Happ's SOCKS port when Happ is running,
  otherwise Hiddify. Other VPN probes keep the Hiddify-first order.
- Omit `google.com` and `*.google.com` from the Diagnostics **reachability**
  card in release / `import.meta.env.PROD` builds. Debug builds keep the
  probe. Live connections and Dashboard packets show whatever Mihomo is
  handling, including a user pin on `google.com`. Direct Rules always
  shows the pin.

## Consequences

A packaged Connect finds the distro Happ binary without a manual path on
the client card. Production reachability never renders the Google probe;
live connections still show Google hosts when they have traffic so a pin
can be verified.
