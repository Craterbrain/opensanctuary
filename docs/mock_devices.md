# Mock devices

The Android TV app, the Roku app and a volunteer's phone are hard to put in front of an automated test. The
mock devices in `web/tests/mock_devices/` speak the same protocols (docs/CLIENT_PAIRING.md) against a real
server, so pairing, certificate pinning, the display socket and remote control can be tested, and driven by a
local agent, without hardware.

| File | Plays |
| --- | --- |
| `mock_tv.ts` | `apps/android-tv`: `PairingManager.kt` and the connect state machine in `MainActivity.kt`. Preferences live in a plain object, so a reboot is a new `MockTv` on the same prefs. |
| `mock_phone.ts` | `/remote`: token verify, commands over `/ws` in the `{cmd, device_token}` envelope, REST fallback. Also the technician's scan-and-authorize step. |
| `console_client.ts` | The operator console, as far as devices care: host token, pairing sessions, remote tokens, revoke. |
| `pinned_tls.ts` | The TV's certificate pinning (trust on first use, or a fingerprint from the ADB intent). |
| `fleet.ts`, `fleet_server.ts` | Many named TVs and phones, a JSON control API for an agent, and `check`: does every connected device show what the console says is live? |

## Tests

`web/tests/e2e_mock_devices.test.ts` covers QR pairing (with and without an ADB-installed pin), one-time token delivery, rejected
session tokens, revoking a remote mid-service, a server restart keeping TVs paired (same certificate and
identity), a different console at the same address (pin mismatch, wrong-console, reset pairing), and the
fleet control server.

## Driving them from an agent

```
bun web/tests/mock_devices/fleet_server.ts --console http://127.0.0.1:8080 --https-port 8443 --listen 9500
```

Endpoints are listed at the top of `fleet_server.ts`. A typical loop: add TVs and a remote, pair them, run the
console through the UI (or `POST /remote/send`), then `POST /check` and read `problems`.

## What this does not cover

The Kotlin and BrightScript code, rendering inside the TV's WebView, mDNS on a real network, and the Android
keystore. Keep one real-device pass (pair, show a slide, reconnect) before each release.
