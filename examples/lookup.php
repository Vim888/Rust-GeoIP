<?php

declare(strict_types=1);

/** Returns the lookup result, or null for an invalid IP or an unavailable service. */
function geoipLookup(string $ip): ?array
{
    if (filter_var($ip, FILTER_VALIDATE_IP) === false) {
        return null;
    }

    $socket = @stream_socket_client('unix:///run/rust-geoip/lookup.sock', $errno, $errstr, 0.2);
    if ($socket === false) {
        return null;
    }
    try {
        stream_set_timeout($socket, 0, 200_000);
        $request = $ip . "\n";
        if (@fwrite($socket, $request) !== strlen($request)) {
            return null;
        }
        $line = @fgets($socket, 16384);
        if ($line === false || !str_ends_with($line, "\n")) {
            return null;
        }
        $result = json_decode($line, true);
        return is_array($result) && array_key_exists('country_code', $result) ? $result : null;
    } finally {
        fclose($socket);
    }
}
