# ADR 0095: Preserve Helper identity across package upgrades

- Status: Accepted
- Date: 2026-09-25

## Context

The Debian `prerm` hook runs for upgrades as well as removals. Disabling the
service unconditionally can leave the new application without its Helper.
Package managers invoked from a GUI or through `pkexec` may not provide
`SUDO_UID`, `SUDO_GID`, and `SUDO_USER`; inferring an authorized account from
missing or inconsistent environment values would create the wrong service
identity. Windows NSIS hooks also need to propagate the privileged helper's
exit code instead of reporting package success after an installation error.

## Decision

- On Debian upgrades, keep the existing Helper active until `postinst` runs.
- If a valid root-owned Helper configuration exists, preserve its authorized
  user, staging directory, socket, and TUN name. Atomically update only the
  packaged Mihomo digest, restart the service after unpacking, and require it
  to be active before `postinst` succeeds.
- On a fresh Debian install, automatically provision only when all `SUDO_*`
  identity fields agree with the local passwd entry and identify a non-root
  user. Otherwise finish package installation with an explicit instruction to
  authorize Helper from the app.
- Stop and disable the Debian Helper only for package removal/deconfiguration.
- Check both NSIS Helper install and uninstall process results. A non-zero,
  timeout, or process-start error aborts the corresponding package operation.

## Consequences

Upgrades preserve the desktop identity and report service restart failures to
the package manager. Fresh installs through package managers that cannot
identify a desktop user require the app's authorized Install Helper action.
Native E2E verification remains required on clean Windows and Linux systems.
