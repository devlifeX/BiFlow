# Architecture decision records

Keep this index current. Add a new ADR for each non-obvious change, or update the existing ADR when the decision evolves.

| ID                                                             | Title                                          | Status     |
| -------------------------------------------------------------- | ---------------------------------------------- | ---------- |
| [0001](./0001-record-architecture-decisions.md)                | Record architecture decisions                  | Accepted   |
| [0002](./0002-single-source-version.md)                        | Single-source application version              | Accepted   |
| [0003](./0003-testing-strategy.md)                             | Unit and e2e testing strategy                  | Accepted   |
| [0004](./0004-in-app-third-party-install.md)                   | In-app Hiddify and Mihomo install              | Accepted   |
| [0005](./0005-cloud-rule-fail-safe.md)                         | Fail-safe Iran rule updates                    | Superseded |
| [0006](./0006-linux-windows-packages.md)                       | Linux and Windows release packages             | Accepted   |
| [0007](./0007-done-gate-builds.md)                             | Build-and-test done gate                       | Accepted   |
| [0008](./0008-tauri-async-runtime.md)                          | Explicit Tauri async runtime                   | Accepted   |
| [0009](./0009-user-facing-readme.md)                           | User-facing README                             | Accepted   |
| [0010](./0010-runtime-observability.md)                        | Runtime and network observability              | Accepted   |
| [0011](./0011-logo-derived-palette.md)                         | Logo-derived palette                           | Accepted   |
| [0012](./0012-tag-releases-and-deny.md)                        | Atomic tag releases and deny policy            | Accepted   |
| [0013](./0013-session-debug-log.md)                            | Permanent structured debug log                 | Accepted   |
| [0014](./0014-windows-ci-line-endings.md)                      | Windows CI line endings and fsync              | Accepted   |
| [0015](./0015-dev-helper-noexec-run.md)                        | Dev helper binaries off noexec /run            | Accepted   |
| [0016](./0016-mihomo-relative-rule-paths.md)                   | Mihomo relative rule-provider paths            | Accepted   |
| [0017](./0017-empty-custom-providers-ready.md)                 | Empty custom providers are ready               | Accepted   |
| [0018](./0018-hiddify-egress-before-tun.md)                    | Probe Hiddify egress before TUN                | Accepted   |
| [0019](./0019-clippy-cfg-tail-expressions.md)                  | Clippy cfg tail expressions                    | Accepted   |
| [0020](./0020-native-linux-windows-ci.md)                      | Native Linux and Windows CI                    | Accepted   |
| [0021](./0021-shared-build-contract.md)                        | Shared local/GitHub build contract             | Accepted   |
| [0022](./0022-biflow-owned-rules.md)                           | BiFlow-owned rule distribution                 | Accepted   |
| [0023](./0023-basic-mode-persistence.md)                       | Basic mode UI persistence                      | Accepted   |
| [0024](./0024-signed-github-release-updater.md)                | Signed GitHub Release updater                  | Superseded |
| [0025](./0025-paused-lifecycle.md)                             | Paused lifecycle and Hiddify keep              | Accepted   |
| [0026](./0026-linux-webkit-blank-view.md)                      | Linux WebKit blank-view workaround             | Accepted   |
| [0027](./0027-windows-debug-log-clear.md)                      | Windows debug.log clear and tests              | Accepted   |
| [0028](./0028-embedded-windows-rules.md)                       | Embedded Iran rules for portable Windows       | Accepted   |
| [0029](./0029-packaged-helper-install.md)                      | Packaged privileged helper install             | Accepted   |
| [0030](./0030-no-background-console.md)                        | No background console or terminal              | Accepted   |
| [0031](./0031-local-github-actions-mirror.md)                  | Local GitHub Actions mirror                    | Accepted   |
| [0032](./0032-fresh-hiddify-start.md)                          | Fresh Hiddify start                            | Accepted   |
| [0033](./0033-windows-platform-backend.md)                     | Windows platform backend                       | Accepted   |
| [0034](./0034-bidirectional-route-pins.md)                     | Bidirectional route pins                       | Accepted   |
| [0035](./0035-windows-reveal-and-helper-install.md)            | Windows debug.log reveal and helper install    | Accepted   |
| [0036](./0036-windows-helper-task-xml.md)                      | Windows helper scheduled task XML              | Accepted   |
| [0037](./0037-windows-mihomo-controller-reachability.md)       | Windows Mihomo controller reachability         | Accepted   |
| [0038](./0038-windows-tun-readiness.md)                        | Windows TUN readiness and clash alignment      | Accepted   |
| [0039](./0039-complete-update-channels.md)                     | Complete update channels                       | Accepted   |
| [0040](./0040-reliable-update-check.md)                        | Reliable update check                          | Superseded |
| [0041](./0041-reliable-cloud-rule-sync.md)                     | Reliable cloud rule sync                       | Accepted   |
| [0042](./0042-responsive-bottom-nav.md)                        | Responsive bottom navigation                   | Accepted   |
| [0043](./0043-persistent-traffic-totals.md)                    | Persistent traffic totals                      | Superseded |
| [0044](./0044-connect-installs-dependencies.md)                | Connect installs required services             | Accepted   |
| [0045](./0045-square-connection-glow.md)                       | Square connection glow                         | Accepted   |
| [0046](./0046-persist-window-size.md)                          | Persist window size                            | Accepted   |
| [0047](./0047-three-viewport-layouts.md)                       | Three representative viewport layouts          | Accepted   |
| [0048](./0048-input-context-menu.md)                           | Input context menu                             | Accepted   |
| [0049](./0049-state-aware-tray-menu.md)                        | State-aware tray menu                          | Accepted   |
| [0050](./0050-connection-operation-lock.md)                    | Connection operation lock                      | Accepted   |
| [0051](./0051-button-icons.md)                                 | Icons on every button                          | Accepted   |
| [0052](./0052-in-button-connection-progress.md)                | In-button connection progress                  | Accepted   |
| [0053](./0053-connect-button-glow.md)                          | Connect button availability glow               | Accepted   |
| [0054](./0054-curated-iranian-business-domains.md)             | Curated Iranian business domains               | Accepted   |
| [0055](./0055-clear-hiddify-system-proxy.md)                   | Clear Hiddify system proxy on pause/stop       | Accepted   |
| [0056](./0056-live-mihomo-connections.md)                      | Live Mihomo connections in Diagnostics         | Accepted   |
| [0057](./0057-github-releases-in-app-update.md)                | GitHub Releases in-app update                  | Accepted   |
| [0058](./0058-direct-domain-nameserver-policy.md)              | DIRECT domain nameserver policy                | Accepted   |
| [0059](./0059-user-selected-direct-dns.md)                     | User-selected DIRECT DNS resolvers             | Accepted   |
| [0060](./0060-fake-ip-default-direct-dns.md)                   | Fake-ip is the default DIRECT DNS              | Accepted   |
| [0061](./0061-direct-domains-always-skip-fake-ip.md)           | DIRECT domains always skip fake-ip             | Accepted   |
| [0062](./0062-no-hiddify-system-proxy-while-running.md)        | No Hiddify system proxy while running          | Accepted   |
| [0063](./0063-reject-quic-toward-vpn.md)                       | Reject QUIC toward the VPN                     | Accepted   |
| [0064](./0064-windows-machine-wide-helper-staging.md)          | Windows machine-wide helper staging            | Accepted   |
| [0065](./0065-linux-vpn-doh-and-egress-probe.md)               | Linux VPN DoH and multi-path egress probe      | Accepted   |
| [0066](./0066-reachability-diagnostics.md)                     | Reachability section in Diagnostics            | Accepted   |
| [0067](./0067-owned-side-tunnel-invariants.md)                 | OwnedSideTunnel invariants                     | Accepted   |
| [0068](./0068-client-registry-and-egress-kinds.md)             | Client registry and egress kinds               | Accepted   |
| [0069](./0069-named-rule-lists.md)                             | Named rule lists                               | Accepted   |
| [0070](./0070-fail-closed-rule-level.md)                       | Rule-level fail-closed                         | Accepted   |
| [0071](./0071-exact-subdomain-pins.md)                         | Exact subdomain pins, longest match wins       | Accepted   |
| [0072](./0072-side-tunnel-profile-picker.md)                   | Native profile picker for side-tunnel files    | Accepted   |
| [0073](./0073-pending-mihomo-settings-apply.md)                | Pending Mihomo apply after live settings       | Accepted   |
| [0074](./0074-happ-discovery-and-hidden-google.md)             | Happ discovery and hidden Google hosts         | Accepted   |
| [0075](./0075-windscribe-openvpn-download.md)                  | Windscribe also offers the OpenVPN installer   | Accepted   |
| [0076](./0076-local-proxy-live-recovery.md)                    | Live recovery of local-proxy clients           | Accepted   |
| [0077](./0077-multi-client-primary-and-hiddify-requirement.md) | Multi-client primaries and Hiddify requirement | Accepted   |
| [0078](./0078-fast-connect-and-egress-watchdog.md)             | Fast Connect and primary-egress watchdog       | Accepted   |
| [0079](./0079-side-tunnel-helper-ipc-timeout.md)               | Side-tunnel helper IPC timeout                 | Accepted   |
