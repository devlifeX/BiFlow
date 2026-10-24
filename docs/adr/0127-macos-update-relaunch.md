# ADR 0127: Relaunch the replaced macOS app after the old process exits

Status: Superseded by [ADR 0129](./0129-macos-update-downloads-only.md) (6.2.78)

A finished macOS update told the operator to quit and open BiFlow again. The
download had completed and `/Applications/BiFlow.app` may already have been
replaced, but the version on screen did not change.

Closing the window does not quit BiFlow. The close handler hides the window
and leaves the process in the menu bar. Opening the app again is delivered to
that same process by the single-instance plugin, so the operator keeps running
the binary that started the update. Replacing `/Applications/BiFlow.app` while
that process is still mapped is also unreliable, and a copy launched from the
disk image is not the bundle the operator reopens.

The disk image is staged with `ditto` while BiFlow is still open. The
administrator prompt starts a detached helper and returns immediately. BiFlow
then exits the same way AppImage and NSIS updates do. The helper waits until
that process id is gone, checks the staged bundle's
`CFBundleShortVersionString`, swaps `/Applications/BiFlow.app`, and opens that
bundle as the desktop user. It does not launch the updated app as root. A
version mismatch leaves the installed app in place and opens it again.
