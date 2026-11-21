# rust-geoip

[Инструкция на русском](README.ru.md)

Look up a visitor’s IPv4 or IPv6 address on your own Linux server. Your backend sends an IP over a Unix socket and receives one line of JSON with country, city, ASN, organization and ISP. The daemon holds ten GeoIP Legacy databases in memory. Visitor addresses stay on the machine.

**Over 1,000% faster than the standard GeoIP CLI** in both 510-IP benchmarks, even with a fresh socket for each address. The median speedup was **162× on the Linux server** and **13.3× on Debian 13 under QEMU**. [Test results](#test-results)

## One IP lookup per incoming connection

On the test server, starting `geoiplookup` or `geoiplookup6` for each IP took a median of **2,540.6 µs**. The same lookup through the Rust service took **15.7 µs** with a fresh Unix socket and **8.1 µs** with a reused socket. The service timings include receiving and decoding the JSON reply on the client.

The service is already running and the databases are in memory. An external RPC also requires a network round trip to the provider. Its latency depends on the route and the provider’s response time. This benchmark did not time external RPCs.

## Dependencies

| Purpose | Requirements on Debian 13 |
| --- | --- |
| Build from source | Rust 1.85+, Cargo, a C toolchain (`build-essential`) and the GeoIP Legacy headers (`libgeoip-dev`). Cargo reads the Rust dependency versions from `Cargo.lock`. |
| Run the service | Linux with systemd, a Unix socket and `libGeoIP.so.1` (`libgeoip1t64`, installed with `libgeoip-dev`). All ten Legacy database files are required. |
| Install and update the databases | Root access for installation, Bash, GNU coreutils, `getent` (`libc-bin`), `useradd` and `groupadd` (`passwd`), `runuser` and `flock` (`util-linux`). The updater needs `curl`, `ca-certificates`, `gzip` and HTTPS access to the mirror. |
| Optional clients and tests | PHP 8+ for the example, `netcat-openbsd` for an interactive query, Python 3 and `geoip-bin` for the comparison test. |

**The daemon and the PHP example work without Memcached.** libGeoIP loads all ten databases into the daemon’s memory using `GEOIP_MEMORY_CACHE`. A Memcached server and the PHP `memcached` extension are only relevant if your site uses them for its own cache.

## Install on Debian 13

Download and extract the repository, or clone it. Run these commands from the directory containing `Cargo.toml`. Build as your regular user.

```sh
sudo apt update
sudo apt install cargo rustc build-essential libgeoip-dev ca-certificates curl gzip util-linux netcat-openbsd
cargo build --release --locked
sudo bash install.sh
sudo rust-geoip ping
printf '8.8.8.8\n2001:4860:4860::8888\n' | sudo nc -N -U /run/rust-geoip/lookup.sock
```

The installer creates the `rust-geoip` service account, downloads and validates the databases under `/var/lib/rust-geoip`, enables the systemd unit and waits for it to answer. A successful ping prints the release date of the oldest database in memory. The test dataset occupied about 135 MiB on disk and used about 140 MiB of RAM. Reloading briefly held two sets in memory and peaked at 286 MiB. Allow disk space for both sets and the archive being unpacked.

## Connect a website

Grant socket access to the **worker account** that runs your backend. For a Debian PHP-FPM pool running as `www-data`, for example:

```sh
sudo usermod -aG rust-geoip www-data
sudo systemctl restart php8.4-fpm
printf '8.8.8.8\n' | sudo -u www-data nc -N -U /run/rust-geoip/lookup.sock
```

Use your own pool account and unit name as appropriate. Restart the workers so they pick up the new group. Copy [examples/lookup.php](examples/lookup.php) beside your PHP 8+ application code:

```php
require_once __DIR__ . '/lookup.php';

$geo = geoipLookup($_SERVER['REMOTE_ADDR'] ?? '');
$country = $geo['country_code'] ?? null;
```

If a reverse proxy sits in front of the site, configure the web server to resolve the visitor address from your trusted proxy. The example returns `null` if the address is invalid or the service is unavailable.

## Socket protocol

Connect to `/run/rust-geoip/lookup.sock`. Send a numeric address followed by LF, then read through the next LF. One connection can carry multiple requests. The daemon closes the connection after 10 seconds idle or a line exceeding 64 bytes including the terminator.

| Request | Reply |
| --- | --- |
| IPv4 or IPv6 | JSON object with `country_code`, `country_name`, `city`, `asn`, `organization`, `isp`. Missing records are `null`. |
| Invalid address | `{"error":"invalid_ip"}` |
| `PING` | `PONG YYYY-MM-DD` |
| `RELOAD` | `OK`, `BUSY`, `ERROR` or `DENIED`. Only root and the service account can reload. |

When present, `city` contains country codes and name, region, city name, postal code, time zone, coordinates and area/metro codes. The default provider derives organization and ISP from ASN data, so these fields often match. [Database provider](https://mailfud.org/geoip-legacy/)

## Update and check

```sh
systemctl list-timers rust-geoip-update.timer
sudo systemctl start rust-geoip-update.service
sudo rust-geoip ping
sudo -u rust-geoip rust-geoip validate /var/lib/rust-geoip
sudo journalctl -u rust-geoip -n 30 --no-pager
```

The updater downloads over HTTPS, checks gzip integrity and validates all ten databases before publishing the set atomically. A failed download leaves the previous set in place. A running daemon reloads while serving lookups. On a reload error it keeps its previous in-memory set. Check the journal and retry with `sudo systemctl reload rust-geoip`.

The installer enables a weekly systemd timer. To manage updates manually, run `sudo systemctl disable --now rust-geoip-update.timer`. Download errors appear in `journalctl -u rust-geoip-update`.

To use your own HTTPS mirror, set `GEOIP_SOURCE_URL=https://mirror.example.org/geoip` in `/etc/default/rust-geoip` before installation or the next update. It must serve `GeoIP`, `GeoIPCity`, `GeoIPASNum`, `GeoIPOrg` and `GeoIPISP`, each with both `.dat.gz` and `v6.dat.gz` suffixes and the same dated mailfud Legacy format.

## Test results

### Linux server benchmarks — Intel Core i7-14700F, 64 GB DDR5

**Rust service and standard CLI.** A sequential run on the same 510 IPv4 and IPv6 addresses returned matching results for all five databases per address family. The service timings include sending the request and decoding the JSON reply, plus opening the socket in the fresh-connection case. The CLI timing includes process startup.

| Lookup method | Median (µs) | p95 (µs) | Speed increase over CLI |
| --- | ---: | ---: | ---: |
| `geoiplookup` / `geoiplookup6`, new process per IP | 2,540.6 | 2,855.6 | 0% (baseline) |
| Rust, fresh socket per IP | 15.7 | 20.2 | **+16,082%** |
| Rust, reused socket | 8.1 | 15.4 | **+31,265%** |

The speed increase uses median lookup times from the same run and equals `(CLI time / method time − 1) × 100%`. A lookup that takes half as long is 100% faster.

**Comparison of service implementations.** The C++, Python, Go and Rust prototypes were long-running processes using the same `libGeoIP` library and ten in-memory databases. Full replies matched on 1,507 test inputs. After warm-up, each connection mode ran 24,112 requests with one client and 96,448 with eight. Each cell shows **median / p95 in microseconds**, including JSON decoding by the client.

| Clients and connections | C++ | Python | Go | Rust |
| --- | ---: | ---: | ---: | ---: |
| 1 client, reused socket | 12.9 / 16.1 | 25.8 / 33.8 | 11.0 / 18.4 | 9.4 / 12.1 |
| 1 client, fresh socket per IP | 18.1 / 36.2 | 76.3 / 87.8 | 19.9 / 51.5 | 15.7 / 20.8 |
| 8 clients, reused sockets | 38.9 / 61.3 | 156.7 / 170.1 | 26.0 / 58.3 | 21.0 / 31.1 |
| 8 clients, fresh socket per IP | 68.0 / 93.0 | 416.3 / 456.8 | 51.5 / 97.1 | 46.0 / 61.1 |

**PHP client measurements.** With a fresh connection to the Rust service, median latency was about 14 µs and p99 ranged from 20 to 48 µs across runs. Processing the IP inside the daemon took about 3 µs. In a separate load test a single request-handling thread reached about 140,000 lookups/s at saturation. The daemon also accepted 20,000 connections that sent no requests and continued serving lookups.

### Current version for Debian 13

All **510 of 510** IPv4 and IPv6 lookups matched `geoiplookup` or `geoiplookup6` on the same database files. The test also confirmed that a fresh socket and a reused socket returned identical JSON values. These timings were measured on an x86-64 Debian guest under QEMU software emulation.

| Request path | Median (µs) | p95 (µs) | Speed increase over CLI |
| --- | ---: | ---: | ---: |
| CLI process per address | 11,759.5 | 13,485.0 | 0% (baseline) |
| Python client to Rust service, fresh socket | 885.0 | 1,271.6 | **+1,229%** |
| Python client to Rust service, reused socket | 507.1 | 844.5 | **+2,219%** |

The service and weekly timer started after a reboot. A failed mirror download preserved the active database set. A successful update switched to the new set while the daemon kept the same process ID.

To measure on your own host, run the following from the repository directory between database updates. The script uses 10 fixed addresses and 500 reproducible pseudorandom addresses, and reports the median and 95th percentile for each path. [Test script](tests/check-lookup.py)

```sh
sudo apt install python3 geoip-bin
sudo python3 tests/check-lookup.py
```

## License and data

The code is distributed under the [MIT License](LICENSE). The installer fetches data separately. The default mirror converts MaxMind GeoLite2 data and refers users to the GeoLite terms. [Database source](https://mailfud.org/geoip-legacy/) · [MaxMind GeoLite EULA](https://www.maxmind.com/en/geolite/eula)
