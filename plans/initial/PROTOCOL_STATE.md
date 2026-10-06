# Protocol, state and mutation receipts

Scope: authoritative proposed Goatr v1 types, framing, reducers, bounded storage and action semantics. These are Goatr contracts, not upstream APIs. Implement `protocol/goatr-v1.schema.json`, shared fixtures, Kotlin `Protocol.kt` and Python `protocol.py` before adapters.

[Main plan](GOATR_PLAN.md) · [Native transport](TECH_STACK.md) · [Pairing/security](PAIRING_SECURITY.md) · [Herdr identity](HERDR_INTEGRATION.md) · [Testing](TESTING.md)

## Framing and value conventions

One UTF-8 JSON object per 4-byte unsigned big-endian length-prefixed frame. No compression. See the limits table below. Reject zero/oversized lengths before allocation; reject invalid UTF-8, duplicate keys at every nesting level, unpaired surrogates, nonfinite numbers, wrong types, missing required fields and unknown request/answer fields before dispatch. Python must not accept booleans as integers; Kotlin must not round JSON numbers through Double. Close on framing/structural errors; where a valid request UUID is recoverable return `invalid_request` first. Unknown version returns `unsupported_version` then closes. Unknown method returns `unsupported_method`. Ignore unknown optional response fields; unknown event kind or unknown required enum requires resync/incompatible UI, never optimistic application. Unknown native requests become desktop-only, never an invented decision.

Notation below: `?` means optional/omitted, not null. Objects are closed except explicitly named maps; arrays preserve order; all fields are required otherwise. UUID means canonical lowercase RFC 4122 UUID text, generated with random v4 for Goatr identities. `Id` is an opaque nonempty native string, never executable/path authority. `Uint` is a JSON integer 0..9007199254740991; timestamps are Unix milliseconds within that range, absent when upstream gives no trustworthy time. Native numeric JSON-RPC IDs stay native at the adapter, with their original integer/string type; Android gets only `requestKey`. `Target={sessionId:UUID,generation:UUID}`. Endpoint IDs use iroh's canonical library parser/formatter, never a display name. `Text` is valid Unicode; lengths below count UTF-8 bytes except explicitly stated Unicode scalar counts.

Only integer JSON numbers occur in Goatr structured types. MCP numeric values/defaults/ranges use decimal strings: `-?(0|[1-9][0-9]*)(\.[0-9]+)?`, no exponent, no leading plus, strip fractional trailing zeroes, normalize negative zero to `"0"`; at most 128 bytes. Compare using exact decimal arithmetic; convert to native JSON numeric tokens only in the Codex adapter. Native JSON preserved for display is a UTF-8 string/content reference, not an arbitrary number-bearing Goatr object.

Envelopes:
- Request `{v:1,type:"request",id:UUID,method:Method,params:object}`.
- Response `{v:1,type:"response",id:UUID,result:object}` OR `{v:1,type:"response",id:UUID,error:Error}`.
- Event `{v:1,type:"event",subscription:UUID,seq:Uint,kind:EventKind,data:object}`. Subscription sequences are volatile; notification sequences below are durable and distinct.

## Exact object vocabulary

| Type | Fields / finite alternatives |
|---|---|
| Backend | `{provider:"omp"\|"codex",version:Text,availability:"ready"\|"setup_required"\|"incompatible"\|"unavailable",reason?:Text}` |
| Server | `{serverId:UUID,name:Text,connectivity:"online"\|"offline"\|"unavailable",canCreate:{"omp":boolean,"codex":boolean},reason?:Text}` |
| Project | `{projectId:UUID,label:Text,available:boolean,reason?:Text}`; path stays host-local, projects govern creation only |
| Capabilities | `{history:boolean,prompt:boolean,cancel:boolean,respond:boolean,promptMode:"submit_or_steer"\|"unavailable",cancelMode:"current_work"\|"native_turn"\|"unavailable"}`; false is authoritative |
| HistoryCoverage | `{scope:"all_branches"\|"thread"\|"unavailable",source:"persisted_and_live"\|"live_only"\|"native_pages"\|"unavailable",olderAvailable:boolean,sourceTruncated:boolean,desktopBranchKnown:boolean}` |
| SessionSummary | `{sessionId:UUID,generation:UUID,serverId:UUID,workspaceId:Id,terminalId:Id,paneId:Id,provider:"omp"\|"codex",nativeId:Id,title:Text,status:"idle"\|"working"\|"attention"\|"unknown",connectivity:"ready"\|"synchronizing"\|"offline"\|"setup_required"\|"unavailable",capabilities:Capabilities,historyCoverage:HistoryCoverage,reason?:Text}` |
| UnboundRow | `{rowId:UUID,serverId:UUID,workspaceId:Id,terminalId:Id,paneId:Id,provider:"omp"\|"codex",title:Text,state:"setup_required"\|"ambiguous"\|"unavailable",reason:Text}`; connection-local row identity, **no** fabricated sessionId/generation/nativeId and no mutation target |
| SessionState | `{summary:SessionSummary,nativeTurnId?:Id,desktopSelection:"unknown"\|"bound_launch_thread",activity:Activity}` |
| Activity | `{state:"idle"\|"working"\|"unknown",text?:Content,toolRecordId?:Id}`; transient, not a completed history record |
| Content | `{kind:"text",text:Text}` OR `{kind:"ref",handle:UUID,byteLength:Uint,mime:"text/plain"\|"application/json"}` OR `{kind:"unavailable",reason:"source_truncated"\|"not_cached"\|"unsupported"\|"too_large",byteLength?:Uint}` |
| HistoryRecord | `{id:Id,parentId?:Id,turnId?:Id,order:Uint,timestamp?:Uint,role:"user"\|"assistant"\|"tool"\|"system"\|"unknown",kind:"message"\|"tool"\|"notice"\|"unsupported",text?:Content,tool?:Tool,original?:Content,source:"persisted"\|"live"\|"native_page",complete:boolean,sourceTruncated:boolean}` |
| Tool | `{name:Text,callId?:Id,status:"running"\|"succeeded"\|"failed"\|"unknown",input?:Content,output?:Content}`; no terminal escape execution |
| PendingRequest | `{requestKey:UUID,provider:"omp"\|"codex",kind:Presentation.kind,presentation:Presentation}`; bound internally to exact native request ID/type and Target; no generic responseSchema interpreter |
| Error | `{code:ErrorCode,message:Text}`; fixed safe message, no provider payload/token/path leakage |

`order` is reducer-assigned stable order within the current generation, not a fabricated native ID or durable upstream cursor. Preserve native IDs/parent links; unknown native history entries retain an `unsupported` record and safe original content, not a guessed assistant message. OMP records use file order then native live append order, displaying parent/branch cues; Codex orders turns/items using native ordering, not phone arrival time.

Finite `Presentation` / `answer` pairs (provider source-specific projection is in [OMP](OMP_BACKEND.md) and [Codex](CODEX_BACKEND.md#native-approvals-and-questions)):
- `omp_select`: `{kind,title:Content,options:[{value:Text,label:Text,description?:Content}],selected?:Uint,checked:[Uint]}` → `{kind:"omp_select",action:"submit",value:Text}` or `{kind:"omp_select",action:"cancel"}`. Selection must be an offered value; checked markers do not invent multiselect support.
- `omp_editor`: `{kind,title:Content,prefill:Content}` → `{kind:"omp_editor",action:"submit",text:Text}` or `{kind:"omp_editor",action:"cancel"}`.
- `codex_approval`: `{kind,approvalType:"command"|"file"|"permissions",title:Content,details:Content,choices:[{choiceId:Id,label:Text,scope:"once"|"turn"|"session"|"policy",details?:Content}]}` → `{kind:"codex_approval",choiceId:Id}`. Host stores exact offered native results in memory; phone cannot supply/amend grant JSON.
- `codex_questions`: `{kind,isBlocking:boolean,questions:[{id:Id,header:Text,prompt:Content,isOther:boolean,isSecret:boolean,options:[{label:Text,description:Content}]}]}` → `{kind:"codex_questions",answers:[{id:Id,values:[Text]}]}`; unique exact question IDs, one answer per question, offered labels unless free input is native-supported. No fabricated cancel API.
- `codex_form`: `{kind,title:Content,fields:[FormField]}` → `{kind:"codex_form",action:"accept",values:[{id:Id,value:FieldValue}]}` or `{kind:"codex_form",action:"decline"|"cancel"}`. `FormField={id,title:Text,description?:Content,required:boolean,type:"string"|"number"|"integer"|"boolean"|"single"|"multi",default?:FieldValue,format?:"email"|"uri"|"date"|"date-time",minLength?:Uint,maxLength?:Uint,minimum?:decimalString,maximum?:decimalString,minItems?:Uint,maxItems?:Uint,options?:[{value:Text,label:Text}]}`. Only constraints meaningful to the declared type are allowed. `FieldValue` is a string (including exact decimal for numeric types), boolean, or array of strings for multi. Validate required/unique IDs, format, scalar length, range, membership and cardinality on both sides.
- `codex_url`: `{kind,title:Content,url:Text}` → `{kind:"codex_url",action:"accept"|"decline"|"cancel"}`. Opening a URL is separate from answering and does not close the card.
- `desktop_only`: `{kind,title:Content,reason:Text}`; no answer accepted. Unsupported/oversized/secret-bearing raw request data is never hidden behind an actionable partial form.

## Methods and exact params/results

`bootId:UUID` is additionally REQUIRED in every row marked B. `target` is a current `Target`; no raw provider IDs, executable paths, Herdr methods or shell commands are accepted. Request UUID is action ID for rows marked A. All operations except pair.confirm require paired identity; hello must complete before other authenticated operations.

| Method | B/A | Params beyond bootId | Exact result |
|---|---|---|---|
| pair.confirm | – | `{token:Text,deviceName:Text}` | `{hostId:<EndpointId>,deviceId:UUID,paired:true}` |
| hello | – | `{appVersion:Text,deviceName:Text}` | `{hostId:<EndpointId>,deviceId:UUID,bootId:UUID,protocol:1,backends:[Backend]}` |
| host.snapshot | – | `{}` | `{subscription:UUID,seq:0,servers:[Server],projects:[Project],sessions:[SessionSummary],unbound:[UnboundRow]}`; atomically replaces this connection's previous host subscription |
| session.open | – | `{target}` | `{subscription:UUID,seq:0,state:SessionState,pendingRequests:[PendingRequest],history:[HistoryRecord],nextCursor?:Id}` |
| session.history | – | `{target,cursor?:Id,limit?:Uint}` | `{records:[HistoryRecord],nextCursor?:Id}`; default/latest page100, limit1..100 |
| session.unsubscribe | – | `{subscription:UUID}` | `{unsubscribed:true}`; accepts host/session subscription, repeated removal succeeds, never stops agent |
| session.create | B/A | `{serverId:UUID,projectId:UUID,provider:"omp"\|"codex",name:Text}` | `{receipt:Receipt}` including reserved/running creation on timeout |
| prompt.submit | B/A | `{target,text:Text}` | `{receipt:Receipt}` |
| turn.cancel | B/A | `{target,nativeTurnId?:Id}` | `{receipt:Receipt}`; turn ID required Codex, forbidden OMP |
| request.respond | B/A | `{target,requestKey:UUID,answer:<union above>}` | `{receipt:Receipt}` |
| action.status | – | `{actionId:UUID}` | `{receipt:Receipt}` or `{state:"unknown"}` |
| content.read | – | `{target,handle:UUID,offsetBytes:Uint,limitBytes:Uint}` | `{bytes:<standard padded base64>,nextOffsetBytes:Uint,eof:boolean}` |
| notifications.sync | – | `{afterSequence?:Uint,limit?:Uint}` | `{events:[AttentionEvent],read:[ReadWatermark],latestSequence:Uint,retainedFrom:Uint,nextSequence?:Uint,resetRequired:boolean}` |
| notifications.read | B | `{sessionId:UUID,throughSequence:Uint}` | `{read:ReadWatermark}` |
| push.register | B | `{endpoint:Text,endpointGeneration:UUID,pushKey:<base64url 32 bytes>,probe:UUID}` | `{registered:true,endpointGeneration,probe,verified:boolean}` |
| push.probe.ack | B | `{endpointGeneration:UUID,probe:UUID}` | `{verified:true}` |
| push.unregister | B | `{endpointGeneration:UUID}` | `{unregistered:boolean}`; stale generation returns false |
| device.unpair | B | `{}` | `{unpaired:true}` then close; response loss requires local forget, not automatic re-pair |

Read requests may retry with fresh request IDs, including the same still-valid history cursor. Read/ordinary non-agent operation deadline is15 seconds; provider-write boundary timeout is10 seconds; create waits at most120 seconds, then returns its current receipt while the existing job continues. Native bounded operations (e.g. Herdr's30-second launch) record intent before start and are polled without holding the transport reader. Client timeout after reservation is **not** failure/cancellation. No second response is sent; later results arrive via events/action.status. Invalid/expired cursor or handle gives its explicit error. Limit failure never silently clips inventory or an actionable form.

ErrorCode is exactly: `invalid_request`, `unsupported_version`, `unsupported_method`, `unauthorized`, `pairing_failed`, `stale_host`, `stale_session`, `not_found`, `not_ready`, `invalid_answer`, `already_resolved`, `action_id_conflict`, `action_capacity`, `content_expired`, `cursor_expired`, `busy`, `resource_limit`, `rate_limited`, `timeout`, `backend_error`, `push_rejected`, `stale_registration`. `not_found` covers unknown authorized server/project/session; `backend_error` covers a known native rejection only, not unknown dispatch outcome. Post-reservation errors are recorded in Receipt rather than returned as bare errors.

## Identity, ordering and events

[Herdr](HERDR_INTEGRATION.md#durable-identity-and-routing) owns the persistent mapping. Stable sessionId names a proven desktop/native binding; reconnect keeps it while replacing generation. A different native conversation/terminal/owner creates a different sessionId; room replacement of the same OMP process/conversation keeps sessionId but replaces generation. Pane/workspace routing changes alone do neither. Every companion boot has a new bootId and every restored session receives a fresh generation. All old provider handles, content/cursors and pending requests become invalid.

One serialized reducer per session, plus one host/notification reducer and one ordered writer per connection. Snapshot capture, seq0 response enqueue and subscriber activation occur atomically in the owning reducer. Events following that snapshot start at 1. Phones never receive the queued event before its response. Opening a phone subscribes to an existing reducer, not a new native provider subscription. OMP welcome/final snapshot and Codex resume-response/queued-notification boundaries install first; queue subsequent native events until installation completes.

| Event kind | Exact data / subscription |
|---|---|
| host.changed | `{}` / host; inventory invalidation, call host.snapshot and replace old subscription |
| session.state | `{state:SessionState}` / session |
| history.upsert | `{records:[HistoryRecord]}` / session; one or more complete projected records |
| activity.update | `{activity:Activity}` / session; transient only |
| request.opened | `{request:PendingRequest}` / session |
| request.closed | `{requestKey:UUID,reason:"resolved"|"cancelled"|"generation_changed"|"desktop_only"}` / session; not proof this phone won |
| action.status | `{receipt:Receipt}` / host, only originating peer |
| notification.upsert | `{event:AttentionEvent}` / host, only receiving peer |
| notification.read | `{read:ReadWatermark}` / host, only receiving peer |
| sync.required | `{reason:"overflow"|"source_gap"|"generation_changed"}` / either |

On duplicate seq ignore only already-applied events; on a gap stop applying, disable mutations and replace the subscription with a snapshot. On outbound overflow, enqueue sync.required only if it fits without violating order, then close the subscription/connection; otherwise close immediately. Never drop durable/request transitions and continue. Coalesce only transient activity before sequencing. Historical Codex pages may fill missing fields/items but never overwrite newer live revisions; OMP persisted full text takes precedence over collab placeholders for the same ID, while live tool/status transitions take precedence over older file state. Source replacement/truncation invalidates cursors and forces fresh snapshot; never append old-generation material to a new one.

## Mutation receipts

Canonical request hashing: hash UTF-8 canonical JSON of `{method,params}` including bootId/target, excluding envelope id/version. Sort object keys by Unicode scalar order, preserve array order; no whitespace; escape only quote, backslash and U+0000..001F (lowercase `\u00xx`); emit other scalars literally; integers in decimal without sign/leading zero unless negative is schema-allowed (none in v1). No normalization of Unicode or strings. Normalize schema defaults before hashing (only documented omitted limit defaults; action params have none). Since structured noninteger numbers are forbidden, Python/Kotlin need no floating-point canonicalization. Fixtures contain input bytes, canonical bytes and SHA-256 hex. Durable companion fingerprints use HMAC-SHA256 over canonical bytes with a private local receipt key; phone uses its own protected key. This prevents offline guessing of secret answers from stored unkeyed hashes. Do not persist canonical bytes.

Check order: (1) frame/schema/limits; (2) authenticated peer/hello and current authorization epoch; (3) peer-scoped action ID lookup and fingerprint comparison; matching existing receipt is returned **before** stale boot/target checks, with no native dispatch; conflict errors; (4) for an unseen ID validate current boot, target/capability/turn/request and answer; (5) reserve durably under the peer fence; (6) recheck authorization and target at every actual side-effect boundary. Revoked peers cannot use receipt lookup to read anything. `action.status` can retrieve an old-boot receipt after a new hello; unknown/expired status never authorizes resend. A matching receipt query is not a native retry.

`Receipt={actionId:UUID,method:"session.create"|"prompt.submit"|"turn.cancel"|"request.respond",bootId:UUID,target?:Target,createdAt:Uint,updatedAt:Uint,state:"reserved"|"dispatching"|"dispatched"|"succeeded"|"failed"|"uncertain",error?:Error,result?:ActionResult,creation?:CreationProgress}`. No text, answer, original request JSON or grant payload is stored in receipt metadata on either device. Internal records also contain peer ID, authorization epoch and keyed fingerprint, never returned on wire.

Allowed transitions: new→reserved→dispatching→dispatched or succeeded; reserved→failed before any effect; dispatching→failed only with definitive native rejection/no-effect proof for the attempted operation; reserved/dispatching→uncertain after crash/lost outcome; dispatched→succeeded only upon correlated native acknowledgement; uncertain→succeeded/failed only with definitive reconciliation proof, never replay. For multistage creation, dispatching persists across stages: a known later-stage failure can be failed **with all earlier confirmed resources retained**; any unaccounted stage effect instead makes the whole outcome uncertain. Terminal failed/succeeded do not restart. Dispatched is an honest no-ack receipt, not queued retry. Persist dispatching before attempting native write: crash between commit and write is indistinguishable from execution and becomes uncertain. Restart marks unfinished reserved/dispatching actions uncertain, never resumes them automatically.

`ActionResult` is `{kind:"prompt",nativeTurnId?:Id}` or `{kind:"cancel",nativeTurnId?:Id}` or `{kind:"response",resolution:"written"|"acknowledged"|"resolved_elsewhere_or_unknown"}` or `{kind:"created",target:Target,workspaceId:Id,terminalId:Id,paneId:Id,nativeId:Id,bindingId?:UUID}`. OMP writes ordinarily end dispatched; dialog closure alone does not establish the winner. Codex clientUserMessageId correlates but does not guarantee native idempotence. Succeeded means admission/operation acknowledgement, not entire turn completion.

### Ordered creation progress

`CreationProgress={serverId:UUID,projectId:UUID,provider:"omp"|"codex",stage:"workspace"|"owner"|"thread"|"relay"|"launch"|"ready",stageState:"not_started"|"dispatching"|"confirmed"|"uncertain"|"failed",resources:CreationResources}`. `CreationResources={workspaceId?:Id,paneId?:Id,terminalId?:Id,nativeId?:Id,bindingId?:UUID,sessionId?:UUID,generation?:UUID,ownerStarted?:boolean,relayStarted?:boolean}`; omit unknown values rather than report false certainty. Host-only metadata retains socket/process epochs, unit names and per-stage intent/results from the backend contract. Persist stage intent and returned IDs before the next side effect. OMP skips owner/thread and confirms nativeId after launch; Codex follows its ordered steps. Ready/confirmed plus resulting Target is required for create succeeded.

Creation deadline returns `{receipt}` in its actual running state, not false success. Disconnect does not cancel the durable in-process job; companion restart does not replay it. A failed later stage retains earlier resources and reports partial creation; any unconfirmed effect makes the receipt uncertain. Record correlation alias/actionId even if workspace/thread IDs were lost, expose the known resources, inspect read-only native state and require explicit operator reconciliation when proof is insufficient. Never delete the workspace, restart a launch, create another thread, or roll back already-started desktop work silently. Revocation stops subsequent stages; confirmed earlier resources survive.

## Durable attention and read state

The two notification methods are the narrow addition required by live/push dedup, not a transcript sync framework. `AttentionEvent={eventId:UUID,sequence:Uint,sessionId:UUID,generation:UUID,category:"attention"|"completed",createdAt:Uint}`. Allocate and persist eventId/monotonic sequence per paired peer in the same transaction as the authoritative transition: new actionable request/known attention, or working→idle completion. Do not emit completion from disappearance, unknown busy state or initial/reconnected snapshot. Store native transition correlation with the reducer checkpoint to avoid duplicate emission after restart. Event sequences never reset on companion restart or push-key rotation.

`ReadWatermark={sessionId:UUID,throughSequence:Uint}` is per peer/session, monotonic, advanced only through events that peer has received from authenticated sync/live state while actually viewing that session. Reject future/wrong-session sequence with invalid_request. `notifications.sync` returns ascending events afterSequence (default0), a complete bounded read-watermark list, latestSequence and first retained sequence (latest+1 if empty); max/default page100. `nextSequence` is last returned when more pages remain. `resetRequired` says requested history predates retention: refresh current state/read markers, do not invent missed notifications or replay an old backlog as new. Live and push share eventId; push alone neither marks read nor changes transcript. Probes are not AttentionEvents and have no session/sequence.

## Bounds, content and privacy retention

This table is the single numeric authority for shared wire/storage/privacy limits. Security-specific admission and push retry timers are owned by their topics.

| Resource | Limit / eviction |
|---|---|
| Frame / JSON depth / aggregate members | 1 MiB / 32 / 16,384; reject before dispatch |
| Prompt / editor answer / total form answer | 256 KiB each; never truncate a send |
| Generic Id/cursor / label/device name / safe error | 512 bytes / 1–128 Unicode scalars and ≤512 bytes, no controls / 512 bytes |
| Form questions/fields/options | 64 questions/fields; 256 options per field and 1,024 total; too large → desktop_only |
| Inline Content / loaded content | 16 KiB inline; max16 MiB per payload, host aggregate64 MiB/session and256 MiB total LRU; TTL15 minutes or generation end |
| Content read | 1..65,536 bytes; offsets within byteLength, allow offset=end; base64 bytes may split UTF-8, client decodes after assembly |
| History page / inventory | ≤100 records/page within frame; ≤512 proven+unbound rows, 64 servers, 128 projects; capacity errors are visible, not silent truncation |
| Pending requests / subscriptions | 64/session; 32/peer, 1 active host subscription/connection; capacity leaves excess native requests desktop-only |
| Outbound queue | 256 events or4 MiB, whichever first; overflow recovery above |
| Host reducer history | 1,000 projected records/session, 64 MiB total; older records refetched from native history, not silently represented as complete |
| Receipts | 30 days from terminal/uncertain/dispatched update, 10,000/peer; only expired records evicted, action_capacity otherwise; unknown never permits replay |
| Attention events / dedup | 30 days or10,000/peer, oldest first; retain sequence high-water and read-watermarks through eviction; explicit resetRequired |
| Retired push generations | 30 days, max10,000/peer; evict expired only, reject rotation with resource_limit rather than discard unexpired replay protection |
| Android cached history/content | 50 MiB total, 1,000 records/session, 30-day age; evict oldest unviewed session then oldest records, content LRU; active visible content can be reloaded, never mislabel partial cache complete |
| Android recovery drafts | ≤20 drafts and1 MiB total, 7 days; bound to original host/session/generation/action; warn before replacing/expiring recovery, never auto-send |
| Android unsubmitted drafts | rotation memory only; no durable general draft feature |
| Stored pairs/read-watermarks | ≤32 pairs, ≤512 session watermarks per peer; delete obsolete session watermark only after its last event expires |

Content handles are unguessable UUIDs authorized by current peer and Target, never pathnames; pinned while a chunk is being copied, released after completion/timeout. Eviction/TTL/generation/source replacement returns `content_expired`; reopen/refetch the item, not an old handle. Large fields over the per-payload ceiling show unavailable with source/limit explanation, never false full content. No automatic remote images. Opaque cursors are bound to target/source epoch and expire on replacement.

Phone and companion retain only redacted display history, safe original history JSON and receipt metadata. Never persist pending-request originals, secret answers, capability links, tokens, push endpoints/keys in plaintext logs, or raw control frames. Sensitive native history fields are omitted/redacted before caching; secret answers remain memory-only even when uncertain. Recovery drafts apply to ordinary composer text only and survive process death after pre-send persistence; recording a local receipt must not erase them. Erase recovery on authoritative success/dispatch recorded and user-visible outcome; uncertain retains text, with explicit inspection/manual new action only, never auto-retarget or resend. Exclude all Goatr private state/cache/drafts from Android backup/transfer. Offline local forget deletes pairing keys, push registration/key, cache, drafts, receipts, dedup/read state and notifications for that host; it cannot revoke the server allowlist. Show desktop revoke instructions.

## Acceptance gates

[Testing](TESTING.md) maps every section to executable Python/Kotlin fixtures, provider fault injection, instrumentation or owned runtime checks. No Goatr protocol test has been run during this planning revision.
