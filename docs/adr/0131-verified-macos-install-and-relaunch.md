# ADR 0131: Verify and replace the macOS bundle before scheduling a user relaunch

Status: Accepted (6.2.82)

Supersedes [ADR 0129](./0129-macos-update-downloads-only.md). The user requested
an automatic macOS update: downloading and opening a disk image left the old
menu-bar process running and did not install anything.

## Decision

Keep the existing exact-version, architecture, signed-manifest, and package
signature checks. Mount the verified image read-only without opening Finder.
Validate the exact `BiFlow.app`, executable, bundle identifier, and version;
copy it with `ditto` into a unique staging directory beside the installed app,
then validate that copy before touching the installed bundle.

Await the administrator-authorized installation rather than launching a
replacement script and assuming it succeeded. Move the previous bundle to a
backup, rename the validated candidate into place, and verify it there. Restore
the previous bundle if replacement fails. A failed rollback must retain the
backup directory; no cleanup path may delete the only recoverable copy.
Preserve custom installation locations. A disk-image or App Translocation
launch installs into `/Applications/BiFlow.app` instead of trying to overwrite
the read-only launch location.

On macOS the loaded process can finish while its bundle has been renamed.
Only after replacement completes do we schedule an unprivileged `nohup` shell
to wait for that process to exit and open the exact installed bundle. The
desktop reports `restarting` and uses the existing update exit path. A bounded
wait and the launch result are appended as safe structured diagnostic events.
Launching through the administrator shell would start the GUI as root and is
forbidden. Shell argument quoting and AppleScript string encoding remain
separate operations.

Detach the image after success or failure. Disable automatic recursive staging
cleanup before attach starts, so cancellation or failed detach cannot recurse
into a mounted filesystem. Only a positively detached staging tree is removed.

All package installers now record whether they paused an active connection.
On failure, resume only that connection; preserve a preexisting user pause.
About and the mock transport use the same install/restart progress on macOS as
Windows and AppImage. Manual first installs still use the DMG drag layout;
Debian package installation retains its existing manual restart behavior.

## Validation

Cross-platform tests cover destination selection, quoting, and pause intent.
Native macOS tests create and mount a real temporary DMG, install its bundle,
and verify detachment. They also execute the install script on temporary bundles, reject a
version mismatch before replacement, inject a failed swap and failed rollback,
and execute a relaunch watcher against a temporary process and fake launcher.
The latter verifies safe JSON diagnostics for both launch success and failure.
UI unit tests and the primary-flow e2e assert install/restarting rather than
download-only completion. The real administrator prompt and signed release
upgrade still require a physical Mac acceptance test; CI does not establish
that those interactions occurred.

Amendment (6.2.84): the upstream macOS job failed to attach its temporary
fixture while fork CI passed, and the assertion discarded the command error.
Create a uniquely named HFS+ fixture explicitly and include its exit status
and stderr in test failures. Run attachment through the same helper as the
installer. Bound attachment to 60 seconds and detachment to 20 seconds, killing
the command on timeout while preserving the existing mounted-directory cleanup
rule. Production diagnostics contain safe exit codes and operation classes;
raw paths and command stderr remain out of debug.log. A failed command is not
a successful mount, and failures are not hidden by whole-test retries. The
fixture detaches before asserting failures, including command execution errors.
