#!/bin/bash
set -euo pipefail
umask 0077
if [[ $(id -un) != rust-geoip ]] || (( $# > 1 )); then
    echo "Usage: sudo -u rust-geoip /usr/local/sbin/rust-geoip-update [HTTPS_MIRROR_URL]" >&2
    exit 1
fi
source_url=${1:-${GEOIP_SOURCE_URL:-https://mailfud.org/geoip-legacy}}
cd /var/lib/rust-geoip
exec 8>.update.lock
flock -n 8 || { echo "An update is already running" >&2; exit 1; }

set_dir=$(mktemp -d .rust-geoip-set.XXXXXXXX)
cleanup() {
    if [[ $(readlink active || :) != "$set_dir" ]]; then rm -rf -- "$set_dir"; fi
}
trap cleanup EXIT
for base in GeoIP GeoIPCity GeoIPASNum GeoIPOrg GeoIPISP; do
    for family in '' v6; do
        file="$base$family.dat"
        archive="$set_dir/$file.gz"
        (ulimit -f 65536; curl --disable --fail --silent --show-error --location \
            --proto '=https' --proto-redir '=https' --connect-timeout 10 --max-time 120 \
            --max-filesize 67108864 --output "$archive" -- "${source_url%/}/$file.gz")
        (ulimit -f 262144; gzip -dc -- "$archive" > "$set_dir/$file")
        rm -- "$archive"
    done
done
/usr/local/bin/rust-geoip validate "$set_dir"

# Loaders share this directory lock until all ten databases are in memory.
exec 9<.
flock -x 9
previous=$(readlink active || :)
ln -s -- "$set_dir" "$set_dir/next"
mv -Tf -- "$set_dir/next" active
if [[ $previous =~ ^\.rust-geoip-set\.[[:alnum:]]{8}$ && $previous != "$set_dir" ]]; then
    rm -rf -- "$previous"
fi
flock -u 9
if [[ -S /run/rust-geoip/lookup.sock ]]; then
    /usr/local/bin/rust-geoip reload
fi
echo "GeoIP databases updated."
