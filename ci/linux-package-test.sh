#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Tries an installed Linux package of Katna on a headless X server: D-Bus
# starts katna-daemon when katnactl calls it, and Katna Mail opens a
# window, which is saved as a screenshot. Used by
# .github/workflows/linux-packages.yml on each package's own platform.
#
#   KATNACTL="flatpak run --command=katnactl ..." KATNA_MAIL="..." \
#     ci/linux-package-test.sh SCREENSHOT.png
#
# KATNACTL and KATNA_MAIL default to katnactl and katna-mail on PATH. With
# KATNA_DEMO, a tarball from ci/linux-demo-data.sh, Katna starts with its
# made-up mail, unpacked into KATNA_DATA_HOME and KATNA_CONFIG_HOME (by
# default ~/.local/share and ~/.config; a Flatpak or Snap keeps its own).
# It runs in its own dbus-run-session unless KATNA_TEST_SESSION=1 says the
# session bus it has will do (a Snap needs a real systemd user session).
# Needs Xvfb, dbus-run-session, gnome-keyring-daemon, xwininfo and
# ImageMagick.
set -eu

shot=$1
if [ -z "${KATNA_TEST_SESSION:-}" ]; then
  if [ -z "${XDG_RUNTIME_DIR:-}" ]; then
    XDG_RUNTIME_DIR=$(mktemp -d)
    export XDG_RUNTIME_DIR
  fi
  chmod 700 "$XDG_RUNTIME_DIR"
  export KATNA_TEST_SESSION=1
  # KATNA_DBUS_CONFIG names session.conf where D-Bus has none in /etc (Nix).
  exec dbus-run-session ${KATNA_DBUS_CONFIG:+--config-file="$KATNA_DBUS_CONFIG"} \
    -- "$0" "$@"
fi

katnactl=${KATNACTL:-katnactl}
katna_mail=${KATNA_MAIL:-katna-mail}
log=$(mktemp -d)
trap 'set +e; kill $(jobs -p) 2> /dev/null; pkill -x katna-daemon 2> /dev/null' EXIT

if [ -n "${KATNA_DEMO:-}" ]; then
  data=${KATNA_DATA_HOME:-$HOME/.local/share}
  config=${KATNA_CONFIG_HOME:-$HOME/.config}
  tar -C "$log" -xzf "$KATNA_DEMO"
  mkdir -p "$data" "$config"
  rm -rf "$data/katna" "$config/katna"
  mv "$log/.local/share/katna" "$data/katna"
  mv "$log/.config/katna" "$config/katna"
fi

Xvfb :99 -screen 0 1280x800x24 -nolisten tcp > "$log/xvfb.log" 2>&1 &
export DISPLAY=:99
# Account passwords go to the Secret Service; an unlocked login keyring
# stands in for the desktop's.
mkdir -p "$HOME/.local/share/keyrings"
eval "$(printf test | gnome-keyring-daemon --unlock --components=secrets)"
export GNOME_KEYRING_CONTROL
sleep 2

echo "== katnactl status (D-Bus starts katna-daemon)"
tries=0
until $katnactl status; do
  tries=$((tries + 1))
  if [ $tries -ge 10 ]; then
    echo "katnactl could not reach katna-daemon" >&2
    exit 1
  fi
  sleep 3
done
$katnactl --help > /dev/null

echo "== Katna Mail"
$katna_mail > "$log/mail.log" 2>&1 &
mail=$!
window=
for _ in $(seq 60); do
  sleep 2
  if ! kill -0 "$mail" 2> /dev/null; then
    cat "$log/mail.log"
    echo "Katna Mail quit" >&2
    exit 1
  fi
  window=$(xwininfo -root -tree | grep -i '"katna' | head -n1 || true)
  [ -n "$window" ] && break
done
if [ -z "$window" ]; then
  cat "$log/mail.log"
  xwininfo -root -tree
  echo "Katna Mail opened no window" >&2
  exit 1
fi
echo "window: $window"
# Time to draw its first frames.
sleep 15
kill -0 "$mail"
import -display :99 -window root "$shot"
echo "screenshot: $shot"
# A window that never drew leaves the screen one flat colour.
if [ "$(identify -format %k "$shot")" -le 1 ]; then
  cat "$log/mail.log"
  echo "Katna Mail drew nothing" >&2
  exit 1
fi
$katnactl status
kill "$mail" 2> /dev/null || true
