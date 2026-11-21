#!/bin/bash
set -euo pipefail
cd -- "$(dirname -- "$0")"

if (( EUID != 0 || $# != 0 )); then
    echo "Usage: sudo bash install.sh" >&2
    exit 1
fi
if [[ ! -x target/release/rust-geoip ]]; then
    echo "Build the service first: cargo build --locked --release" >&2
    exit 1
fi

if ! getent group rust-geoip >/dev/null; then
    groupadd --system rust-geoip
fi
if ! id -u rust-geoip >/dev/null 2>&1; then
    useradd --system --gid rust-geoip --home-dir /var/lib/rust-geoip \
        --no-create-home --shell /usr/sbin/nologin rust-geoip
fi

install -d -o rust-geoip -g rust-geoip -m 0700 /var/lib/rust-geoip
mkdir -p /usr/local/bin /usr/local/sbin
binary=$(mktemp /usr/local/bin/.rust-geoip.XXXXXXXX)
trap 'rm -f -- "$binary"' EXIT
install -m 0755 target/release/rust-geoip "$binary"
mv -fT -- "$binary" /usr/local/bin/rust-geoip
install -m 0755 geoip_update.sh /usr/local/sbin/rust-geoip-update
install -m 0644 rust-geoip.service rust-geoip-update.service rust-geoip-update.timer /etc/systemd/system/
systemctl daemon-reload

if ! runuser -u rust-geoip -- /usr/local/bin/rust-geoip validate /var/lib/rust-geoip >/dev/null 2>&1; then
    systemctl start rust-geoip-update.service
fi
systemctl enable rust-geoip.service
systemctl restart rust-geoip.service
for attempt in {1..100}; do
    if /usr/local/bin/rust-geoip ping 2>/dev/null; then
        systemctl enable --now rust-geoip-update.timer
        exit 0
    fi
    sleep 0.1
done
echo "No reply from rust-geoip. Check journalctl -u rust-geoip.service" >&2
exit 1
