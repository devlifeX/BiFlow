#!/bin/sh
set -eu

# During dpkg's upgrade sequence the old Helper should continue serving until
# the new files are unpacked and postinst can restart it with the preserved
# machine configuration. Stop/remove the unit only when the package is actually
# being removed or deconfigured.
case "${1:-}" in
  remove|deconfigure)
    if command -v systemctl >/dev/null 2>&1; then
      if systemctl is-active --quiet iran-split-helper.service; then
        systemctl stop iran-split-helper.service
      fi
      if systemctl is-enabled --quiet iran-split-helper.service; then
        systemctl disable iran-split-helper.service
      fi
    fi
    ;;
  upgrade|failed-upgrade|abort-upgrade|*)
    ;;
esac
