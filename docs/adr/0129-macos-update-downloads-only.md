# ADR 0129: macOS update downloads a disk image and does not restart

Status: Accepted (6.2.78)

Supersedes the install half of [ADR 0127](./0127-macos-update-relaunch.md).

About → Install on macOS verifies the signed disk image, then opens it with
`/usr/bin/open`. Finder shows the drag-to-Applications window from
[ADR 0128](./0128-macos-drag-install.md). BiFlow does not mount the image
itself, does not replace `/Applications/BiFlow.app`, does not ask for an
administrator password, does not pause the tunnel, and does not quit.

The running menu-bar process is still the old version. Closing the window
only hides it, and the single-instance plugin delivers a later launch to that
same process. The new version runs after the operator quits BiFlow from the
menu bar and opens the Applications copy. Linux `.deb` still waits for a
manual restart. AppImage and NSIS still hand off to their replace helpers.

Amendment (6.2.80): failure to open the downloaded disk image also preserves
the stack state. The recovery path receives the install kind and resumes a
paused stack only for an in-place installer. A user-paused macOS connection
must remain paused when Finder cannot open the image.
