# Pairing and security

Scope: identity storage, local administration, bounded single-use QR pairing, authorization and revocation fences.

[Main plan](GOATR_PLAN.md) · [Native transport](TECH_STACK.md) · [Protocol/state](PROTOCOL_STATE.md) · [Companion configuration](COMPANION_PACKAGING.md) · [Source anchors](REFERENCES.md#critical-files--anchors)

## Authority and identity storage

Pairing authorizes **all supported same-user Herdr sessions on this host**, including inventory, history, control and notifications outside configured projects. Projects restrict **new session creation only**. Both desktop pairing output and Android confirmation explicitly disclose this host-wide authority. There are no remote roles or per-project/per-device session ACLs in v1. Same-user malicious processes, a compromised provider or an unlocked paired phone are outside the isolation guarantee; unpaired network peers and relays must not read session data.

Implement `pairing.py`, Android `Pairing.kt`/`SecretStore.kt`, and CLI `goatr pair`, `goatr devices`, `goatr revoke DEVICE_ID`. Endpoint keys, receipt HMAC key and SQLite live under `$XDG_STATE_HOME/goatr` (default `~/.local/state/goatr`), private 0700/0600 paths. Android encrypts endpoint/push/receipt keys using AndroidKeyStore AES-GCM, excludes all private app state from backup/transfer, and fails closed if keys cannot be recovered. No new identity silently overwrites a lost one. Follow [shared privacy retention](PROTOCOL_STATE.md#bounds-content-and-privacy-retention); do not log private keys, tokens, capability URLs, endpoints or control payloads.

## Exact local administration

The running companion alone owns the local administration socket, default `$XDG_RUNTIME_DIR/goatr/admin.sock`. `goatr serve`, `goatr pair`, `goatr devices` and `goatr revoke DEVICE_ID` each accept the same local-only `--admin-socket PATH` option, resolving to that exact absolute path; omission retains the production default. It is not TOML, an environment override or a remote setting. Setup continues to use the production default and needs no override; isolated runners launch serve directly.

For either path, require an existing canonical same-UID 0700 parent (serve may create its missing private parent), a same-UID 0600 Unix socket and Linux `SO_PEERCRED` same-UID validation on both accepted admin connections and CLI connections to serve. Reject relative/oversized paths, symlinks, wrong-owner/nonprivate parents and unsafe existing entries before bind/connect; never unlink an unproven live socket or fall back to the production path after an override fails. The smoke runner owns its exclusive parent; only its recorded companion owns the socket. CLI does not become a second database/identity owner if serve is absent.

One UTF-8 NDJSON request/response per connection, newline mandatory: `{v:1,id:<UUID>,method:<below>,params:<object>}` → `{v:1,id,result:<below>}` OR `{v:1,id,error:{code,message}}`. Reject duplicate keys/unknown fields; max64 KiB request/response, 5-second read/write deadline, max8 simultaneous admin connections. These methods are local only:

| Method | Exact params | Exact result |
|---|---|---|
| pair.open | `{}` | `{uri:Text,expiresAt:UnixMs}`; opens one pairing window, replaces previous unconsumed window |
| devices.list | `{}` | `{devices:[{deviceId:UUID,endpointId:EndpointId,deviceName:Text,pairedAt:UnixMs,lastSeenAt?:UnixMs}]}`; sorted pairedAt |
| device.revoke | `{deviceId:UUID}` | `{revoked:boolean}`; false if already absent; returns only after fence completion |

Local error codes are `invalid_request`, `unsupported_version`, `unsupported_method`, `unauthorized`, `busy`, `timeout`, `internal_error`; safe messages only. CLI formats the URI/QR/list, never stores a second copy of the token. Optional service start belongs to explicit `goatr setup`; ordinary pair/list/revoke report not running and show `goatr serve` instructions retaining any explicit `--admin-socket PATH`.

## Pairing capability and authenticated handshake

`goatr pair` opens a 300-second pairing window and prints terminal QR plus copyable `goatr://pair/v1?data=<base64url-no-padding(JSON)>`. Decoded fields are exactly `{v:1,host:<EndpointId>,name:<display name>,token:<32 random bytes base64url>,expiresAt:<Unix ms>}`. URI ≤4096 bytes, decoded JSON ≤2048 before parse. Token is exactly43 canonical unpadded base64url characters decoding to32 bytes; validate re-encoding, never permissive padding/extra query fields. Host name/deviceName use the shared label bounds. Expiry on the host uses monotonic time; wall time in QR is informative, not authority. Keep only token hash and expiry in memory; restart closes windows. Use N0 discovery, not permanent embedded IPs.

Android pins the scanned endpoint, displays name/fingerprint, host-wide authority and public-relay metadata disclosure, then connects. Before authorization, no inventory/history/content/subscription/notification/receipt reads occur. Unpaired identity may send only pair.confirm within10 seconds and4096 bytes. Bind a consumed token atomically with a durable paired record to authenticated `Connection.remote_id()`, never the supplied deviceName. Generic pairing_failed covers wrong/expired/reused token and closes. Known peers send hello, including after pair.confirm response loss: if the pairing commit succeeded, hello recovers deviceId; if unauthorized, require a fresh locally opened QR. Do not retry a spent token or trust an unknown hello. Revoked identities must pair anew.

Admission bounds: max8 simultaneous unauthenticated connections, 1 handshake/connection, 5 failed attempts/endpoint/minute and30 failed attempts globally/minute; reject excess without allocating session state. Bound rate-limit memory to256 endpoint entries with60-second expiry; global bucket still applies on churn. Max32 paired identities and1 active authenticated connection/peer, 64 in-flight requests/connection, 60 requests/second with burst120. Authenticate a replacement connection before closing the old one; newest completed hello wins under a per-peer lock, old subscriptions close, already-started native writes retain their receipts. This is connection recovery, not a new authorization epoch. Never leak whether another peer/session/action exists through pre-auth errors.

## Revocation fence

Revoke serializes with dispatch through a per-peer authorization fence. Every queued agent action, externally visible creation stage and not-yet-started HTTPS push rechecks the current paired epoch at its actual dispatch boundary. Removal/epoch increment is transactional. Stop accepting new work immediately; reject reserved/unstarted actions unauthorized, cancel pending push attempts, remove registration keys, close subscriptions/streams and remove peer notification/read state. Retained receipt metadata follows its private retention policy but cannot be queried by the revoked peer.

Fence completion waits only for bounded already-entered write/start boundaries, not native turns or full creation. A hung write times out with uncertain receipt and its provider connection is invalidated before fence completion; it cannot later resume writing. Admin revoke reports success only after those boundaries settle. The self-unpair response is the only final frame allowed after removal, then close even if response delivery fails. Already-started desktop work/resources are not aborted or rolled back. A paired phone can unpair itself only; arbitrary-device administration remains desktop-local.

## QR scanning and paste lifecycle

Reuse only MOTD revision `f348f44d3b5ca747e3a8751f13a277fd601ffd73` QR seams: `invite/QrFrameDecoder.kt:decodeQrFrame`, unbranded `inviteQrBitmap` in `invite/QrCode.kt`, and CameraX permission/lifecycle handling from `ui/invite/QrInviteScannerScreen.kt`. Rename packages and replace IRC invite validation. One acceptance gate handles scan and paste; disposal guards already-posted callbacks as well as camera binding. Every camera frame closes in finally; Back never pairs. Denied/no camera leaves paste available. Invalid/oversized input changes no pairing state and permits retry. Use segno only to encode the desktop QR; catch oversized-QR errors and retain the complete paste URI.

## Acceptance gates

See [protocol and pairing fixtures](TESTING.md#protocol-fixture-parity), [revocation fence](TESTING.md#revocation-fence), [Compose behavior](TESTING.md#compose-behavior), [emulator and camera evidence](TESTING.md#emulator-and-camera-evidence). These are required implementation checks, not recorded passing results.
