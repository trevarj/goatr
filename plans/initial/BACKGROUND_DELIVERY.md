# Background delivery

Scope: explicitly selected persistent or ntfy delivery, durable registration/probe handoff, authenticated hints and notification reconciliation.

[Main plan](GOATR_PLAN.md) · [Tech stack](TECH_STACK.md) · [Pairing/security](PAIRING_SECURITY.md#revocation-fence) · [Protocol/state](PROTOCOL_STATE.md#methods-and-exact-paramsresults) · [UI design](UI_DESIGN.md) · [Companion configuration](COMPANION_PACKAGING.md#configuration-and-trust-roots)

## Persistent foreground service and UnifiedPush

`DeliveryService.kt` is a user-started foreground keeper with an ongoing status notification and Stop action; use the applicable specialUse foreground-service declaration as in MOTD, not an unlimited dataSync claim. Start while the activity is visible. Doze, revoked notification permission, force-stop, workstation sleep and public relay outages are explicit limits, not silently healthy states.

`PushService.kt` uses UnifiedPush connector 3.3.5 callbacks, declared non-exported with `org.unifiedpush.android.connector.PUSH_EVENT`. User chooses ntfy as distributor; register Goatr's own per-host/installation instance. Do not use MOTD registrations, soju WEBPUSH, FCM or a separate Goatr ntfy SSE socket.

## Durable endpoint registration and probe

For each endpoint generation Android first durably encrypts endpoint,32-byte pushKey, probe UUID and generation, then sends push.register. Persistence failure sends nothing. Host persists exact registration before publishing. Same generation+same endpoint/key/probe returns the existing state; changed values under that generation return push_rejected. New generation atomically replaces old registration, cancels old queued jobs/probes, retires its key and starts unverified. Keep a bounded retired-generation set with notification retention so delayed old register cannot resurrect it; connection serialization handles in-flight changes. A stale unregister is a no-op (`unregistered:false`), stale probe ACK returns stale_registration. Revocation removes everything for that peer and fences queued publishers.

Persist host probe deadline/verification state across restart. Publish the same probe every5 seconds until matching authenticated ACK or45 seconds from initial registration, even after HTTP2xx; do not extend deadline on retries/restart. Probe may arrive before register response because Android already persisted its key. Duplicates are silent and can be ACKed again. Android queues the ACK until hello if offline; never background-starts a prohibited foreground service. ACK is accepted only for current generation/probe before deadline (already-verified duplicate ACK remains successful); late unverified ACK fails and requires explicit fresh generation. Timeout keeps existing socket active and displays repair status. Reopening after process death resumes the persisted generation/key; no regenerated key under an old generation.

## Validated publishing and authenticated hints

Default allowed origin `https://ntfy.sh`; self-hosted origins require explicit desktop-local configuration. Parse and compare canonical scheme/host/effective port, require HTTPS, no userinfo/fragments/redirects, preserve path/query exactly, endpoint≤2048 bytes. Default443 only unless the exact port appears in configured origin. Resolve DNS per attempt, reject unspecified/multicast/link-local/loopback/private targets for the public default; explicit locally approved self-host origin may use private addresses, never metadata/link-local addresses. Pin the validated address for that connection while retaining hostname TLS verification/SNI, so re-resolution cannot bypass validation. Do not trust environment proxies or follow redirects to another destination; use normal CA verification, never insecure TLS. Emit only safe origin/status diagnostics, not endpoint secrets.

Publish off the control reader with `Content-Type: application/octet-stream`, `X-UnifiedPush: 1`, total10-second timeout, one active attempt/peer. AttentionEvent is persisted before any live/push delivery; stable per-peer IDs/sequences and dedup/read-watermarks are defined exclusively in [protocol notification state](PROTOCOL_STATE.md#durable-attention-and-read-state). HTTP2xx completes that publication attempt but is not proof of device delivery; authenticated reconnect reconciliation covers missed hints.

Retry transport/429/5xx at5,15,30,60,120,300 seconds, then every300 seconds until1 hour after event creation; honor Retry-After only up to the remaining horizon, never spin. No retry on other4xx;404/410 disables current endpoint, auth/config failure requires visible repair. Drop queued hints whose events became read, expired or whose endpoint/authorization generation changed. Retain event state using the common retention policy even after retry horizon; bounded queue max100 pending event hints/peer, coalesce only duplicate eventId and evict oldest hint with explicit sync-required state, never delete durable event/dedup state. No silent distributor/server/mode switch.

Wire hint≤2 KiB: `G1.<base64url-no-padding UTF-8 JSON>.<base64url-no-padding HMAC-SHA256("G1."+payloadSegment)>`. MAC uses current registration's per-peer32-byte pushKey; host private SQLite and Android encrypted storage only, never a provider capability key. Require canonical base64url and exact32-byte MAC, reject duplicate keys/unknown fields, validate fields only after constant-time MAC check. Two closed payload alternatives:
- Event `{v:1,host:<EndpointId>,device:<deviceId UUID>,endpointGeneration:UUID,sessionId:UUID,eventId:UUID,sequence:Uint,category:"attention"|"completed"}`; copied from durable AttentionEvent.
- Probe `{v:1,host:<EndpointId>,device:<deviceId UUID>,endpointGeneration:UUID,probe:UUID,category:"probe"}`; **no sessionId/eventId/sequence**, no invented session for connectivity testing.

Match current host/device/endpoint generation and persisted probe before accepting. Reject old-key hints after rotation/forget. Event hint is a notification hint only: no prompt/path/tool/approval text, no transcript advancement or request resolution. A valid event uses `(host,device,eventId)` dedup shared with live events, not a second push-only ID. Read-watermark suppression requires authenticated reconciled state; push cannot mark read. Discard replay below known watermark; cold process loads durable dedup first.

## Verified handoff and notification entry

Before closing background iroh, Android must receive current-generation probe and obtain successful push.probe.ack response; if ACK response is lost, retry same ACK and keep socket until verification confirmed. Registration/distributor/permission failure preserves selected mode and shows repair status. Offer explicit socket fallback while activity visible, not automatic service starts. ntfy process death cannot be instantly detected by sleeping Goatr.

Foreground/cold/warm notification entry runs hello→host.snapshot→notifications.sync (all pages)→session.open with current proven Target before controls enable. Old-generation session links may show their offline history and current inventory, but never retarget writes to a replacement conversation. Mark read via notifications.read only when the relevant authoritative session is displayed. Persistent mode uses the same live AttentionEvent IDs and read state. Local forget works offline and cancels local registration/notifications but explicitly cannot revoke server authority; Settings shows desktop revoke instructions.

## Acceptance gates

See [revocation fence](TESTING.md#revocation-fence), [notification UI evidence](TESTING.md#emulator-and-camera-evidence), [background delivery](TESTING.md#background-delivery). These are required implementation checks, not recorded passing results.
