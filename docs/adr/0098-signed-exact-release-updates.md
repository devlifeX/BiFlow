# ADR 0098: Require an exact signed update package before pausing connectivity

- Status: Accepted
- Date: 2026-09-25

## Context

The About-page updater chose a package from GitHub Releases. Its suffix
fallback could select a package for another version, and its direct HTTP
download did not use the existing Tauri updater public key. The release
workflow signed AppImage and NSIS bundles, but not the Debian package.

## Decision

- Accept only the exact `BiFlow_<version>_<arch>` package for the running
  platform; reject zero-size assets and non-stable semantic-version release
  tags. Do not fall back to a similarly named asset.
- Sign the Debian package with the same release key and include its signature
  as `linux-deb-x86_64` in `latest.json`. Require all three signed packages
  before publishing a release.
- Use Tauri's updater download, which verifies the package signature against
  the bundled public key. Compare the signed manifest's version, target, and
  URL with the selected GitHub asset, then check the downloaded byte count and
  stage the verified bytes in a private temporary directory. Only afterward
  may the update workflow pause the active stack.
- A missing, stale, or mismatched signed manifest fails closed and leaves the
  current installation and running stack untouched.
- Recording the pre-upgrade route-pin guard is required before pausing the
  stack. A missing/invalid guard is not silently ignored, and a missing rules
  document leaves the guard in place so repeated startups keep showing the
  recovery state until the document is restored.

## Consequences

An older release without the Debian signature cannot be auto-installed through
this path. New releases must publish a matching signed manifest and assets.
Signing proves package provenance, not successful installation: post-install
Helper/protocol/migration verification and native package E2E remain R5/R6
work.
