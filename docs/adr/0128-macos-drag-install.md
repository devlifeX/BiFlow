# ADR 0128: macOS disk image is a drag-to-Applications install

Status: Accepted (6.2.77)

A macOS install is a disk image the operator opens and drags onto
Applications. Tauri's bundler creates that alias only when it passes
`--app-drop-link`. The positions live in `src-tauri/tauri.macos.conf.json`:
`BiFlow.app` at (180, 170), Applications at (480, 170), and a 660 by 400
window. `packaging/macos/dmg-background.png` draws the arrow between them.

The non-GUI retry used to run `bundle_dmg.sh --skip-jenkins` with only the
file name and the app directory. Finder's layout script is what `--skip-jenkins`
skips, but omitting `--app-drop-link` also omitted the Applications alias, so
a failed AppleScript produced a disk image with nothing to drag onto.
`--skip-jenkins` still skips icon positions. The retry now passes the same
volume name, icon position, Applications link, window size, and background as
the normal bundle. The alias is on the image either way.

The in-app updater opens this same disk image and leaves the running app in
place (ADR 0129). This decision is the disk image a person opens from the
release or from About.
