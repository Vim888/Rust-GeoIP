"""Compare the local GeoIP service with the legacy CLI on the same databases."""

import ipaddress
import json
import random
import socket
import statistics
import struct
import subprocess
from time import perf_counter_ns

from pathlib import Path


DATABASES = Path("/var/lib/rust-geoip")
if (DATABASES / "active").is_symlink():
    DATABASES = DATABASES / "active"
SOCKET = Path("/run/rust-geoip/lookup.sock")


def connect():
    client = socket.socket(socket.AF_UNIX)
    client.settimeout(5)
    client.connect(str(SOCKET))
    return client


def request(client, stream, ip):
    client.sendall(ip.encode("ascii") + b"\n")
    return json.loads(stream.readline())


def f32(value):
    return struct.unpack("f", struct.pack("f", value))[0]


def cli_fields(reply, ipv6):
    country = reply["country_code"]
    city = reply["city"]
    city_line = None
    if city:
        keys = ["country_code", "region"]
        if not ipv6:
            keys.append("region_name")
        keys.extend(["city", "postal_code"])
        city_line = ", ".join([
            *(city[k] or "N/A" for k in keys),
            # The service prints the shortest float form; geoiplookup prints the float with %f.
            f"{f32(city['latitude']):.6f}", f"{f32(city['longitude']):.6f}",
            str(city["metro_code"]), str(city["area_code"]),
        ])
    return {
        "Country": f"{country}, {reply['country_name']}" if country else None,
        "City": city_line,
        "ASNum": reply["asn"],
        "Organization": reply["organization"],
        "ISP": reply["isp"],
    }


def report(label, elapsed):
    ordered = sorted(elapsed)
    print(f"{label}: n={len(ordered)}, median={statistics.median(ordered)/1000:.1f} us, "
          f"p95={ordered[(95*len(ordered)-1)//100]/1000:.1f} us")


def main():
    rng = random.Random(20261031)
    addresses = ["8.8.8.8", "1.1.1.1", "81.2.69.142", "5.255.255.242", "127.0.0.1",
                 "2001:4860:4860::8888", "2606:4700:4700::1111", "2a01:4f8::1", "::1", "2001:db8::1"]
    for _ in range(250):
        addresses.append(str(ipaddress.IPv4Address(rng.getrandbits(32))))
        addresses.append(str(ipaddress.IPv6Address((1 << 125) | rng.getrandbits(125))))
    rng.shuffle(addresses)
    cli, persistent, fresh, replies = [], [], [], []
    with connect() as client, client.makefile("rb") as stream:
        for ip in addresses:
            start = perf_counter_ns()
            reply = request(client, stream, ip)
            persistent.append(perf_counter_ns() - start)
            replies.append(reply)
        assert request(client, stream, "localhost") == {"error": "invalid_ip"}

    for ip, reply in zip(addresses, replies):
        start = perf_counter_ns()
        with connect() as other, other.makefile("rb") as incoming:
            second = request(other, incoming, ip)
        fresh.append(perf_counter_ns() - start)
        assert reply == second, ip

    for ip, reply in zip(addresses, replies):
        executable = "geoiplookup6" if ":" in ip else "geoiplookup"
        start = perf_counter_ns()
        output = subprocess.check_output([executable, "-d", str(DATABASES), ip])
        cli.append(perf_counter_ns() - start)
        actual = {}
        for line in output.decode("latin-1").splitlines():
            label, value = line.split(": ", 1)
            actual[label.split()[1]] = None if value == "IP Address not found" else value
        expected = cli_fields(reply, ":" in ip)
        assert expected == actual, (ip, expected, actual)
    report("CLI", cli)
    report("Socket, new connection", fresh)
    report("Socket, reused connection", persistent)


if __name__ == "__main__":
    main()
