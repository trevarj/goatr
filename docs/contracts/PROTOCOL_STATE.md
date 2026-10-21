# Protocol, state and mutation receipts

Scope: authoritative Goatr v2 types, framing, reducers, bounded storage and action semantics. These are Goatr contracts, not upstream APIs. Implement `protocol/goatr-v2.schema.json`, shared fixtures, Kotlin `Protocol.kt` and Rust `companion/src/protocol.rs` before adapters.

[Documentation](../README.md) · [Native transport](TECH_STACK.md) · [Pairing/security](PAIRING_SECURITY.md) · [Herdr identity](HERDR_INTEGRATION.md) · [Testing](../ACCEPTANCE.md)

## Framing and value conventions

One UTF-8 JSON object per 4-byte unsigned big-endian length-prefixed frame. No compression. See the limits table below. Reject zero/oversized lengths before allocation; reject invalid UTF-8, duplicate keys at every nesting level, unpaired surrogates, nonfinite numbers, wrong types, missing required fields and unknown request/answer fields before dispatch. Booleans are never integers. Rust must validate raw numeric tokens before conversion, rejecting `-0`, exponents and noninteger spellings for Uint; Kotlin must not round JSON numbers through Double. A `serde_json::Value` map or typed derive alone is not recursive duplicate-key validation: use a bounded validating visitor/raw-token pass before schema dispatch, including explicitly allowed nested maps. Close on framing/structural errors; where a valid request UUID is recoverable return `invalid_request` first. Unknown version returns `unsupported_version` then closes. Unknown method returns `unsupported_method`. Ignore unknown optional response fields; unknown event kind or unknown required enum requires resync/incompatible UI, never optimistic application. Unknown native requests become desktop-only, never an invented decision.

Notation below: `?` means optional/omitted, not null. Objects are closed except explicitly named maps; arrays preserve order; all fields are required otherwise. UUID means canonical lowercase hyphenated library-roundtripping text with RFC4122 variant bits. Receivers do not restrict version bits; nil/non-RFC4122 variants and noncanonical spelling are rejected. Goatr generates identities/action IDs with random v4. UUID possession is never authorization. `Id` is an opaque nonempty native string, never executable/path authority. `Uint` is a JSON integer 0..9007199254740991; timestamps are Unix milliseconds within that range, absent when upstream gives no trustworthy time. Native numeric JSON-RPC IDs stay native at the adapter, with their original integer/string type; Android gets only `requestKey`. `Target={sessionId:UUID,generation:UUID}`. Endpoint IDs use iroh's canonical library parser/formatter, never a display name. `Text` is valid Unicode; lengths below count UTF-8 bytes except explicitly stated Unicode scalar counts.

Only integer JSON numbers occur in Goatr structured types. MCP numeric values/defaults/ranges use decimal strings: `-?(0|[1-9][0-9]*)(\.[0-9]+)?`, no exponent, no leading plus, strip fractional trailing zeroes, normalize negative zero to `"0"`; at most128 bytes. Normalize/compare the bounded strings exactly, never through f64/Double or a fixed-precision decimal type that cannot cover the full bound; emit native JSON numeric tokens only in the Codex adapter using validated raw-token support. Preserve native integer/string JSON-RPC IDs and offered approval values without lossy numeric conversion. Native JSON preserved for display is a UTF-8 string/content reference, not an arbitrary number-bearing Goatr object.

Native adapters share `native_json::parse(bytes, Limits)`, which constructs literal maps and exact native JSON number tokens without serde feature-marker reinterpretation. Native signed/integer/string IDs use `NativeRpcId`; finite decimal form values use `Decimal::from_native_token`. Native limits are separate from (and never broaden) phone Uint/1MiB/depth32 limits. Dynamic native maps are never reparsed through `serde_json::Value` deserialization.

The frame cap counts **transmitted UTF-8 JSON bytes**, not canonical hash
expansion. Ordinary compact JSON is valid on the wire; canonical control
escaping can produce more than1MiB of hash input from a valid frame.
Do not apply the wire cap to canonical hash bytes.

Envelopes:
- Request `{v:2,type:"request",id:UUID,method:Method,params:object}`.
- Response `{v:2,type:"response",id:UUID,result:object}` OR `{v:2,type:"response",id:UUID,error:Error}`.
- Event `{v:2,type:"event",subscription:UUID,seq:Uint,kind:EventKind,data:object}`. Subscription sequences are volatile; notification sequences below are durable and distinct.

## Exact object vocabulary

| Type | Fields / finite alternatives |
|---|---|
| Backend | `{provider:"omp"\|"codex",version:Text,availability:"ready"\|"setup_required"\|"incompatible"\|"unavailable",reason?:Text}` |
| Server | `{serverId:UUID,name:Text,connectivity:"online"\|"offline"\|"unavailable",canCreate:{"omp":boolean,"codex":boolean},reason?:Text}` |
| Project | `{projectId:UUID,label:Text,available:boolean,reason?:Text}`; path stays host-local, projects govern creation only |
| Capabilities | `{history:boolean,prompt:boolean,cancel:boolean,respond:boolean,terminalRead?:boolean,worktreesRead?:boolean,changesRead?:boolean,workspaceRename?:boolean,promptMode:"submit_or_steer"\|"unavailable",cancelMode:"current_work"\|"native_turn"\|"unavailable"}`; false is authoritative |
| HistoryCoverage | `{scope:"all_branches"\|"thread"\|"unavailable",source:"persisted_and_live"\|"live_only"\|"native_pages"\|"unavailable",olderAvailable:boolean,sourceTruncated:boolean,desktopBranchKnown:boolean}` |
| SessionSummary | `{sessionId:UUID,generation:UUID,serverId:UUID,workspaceId:Id,terminalId:Id,paneId:Id,provider:"omp"\|"codex",nativeId:Id,title:Text,status:"idle"\|"working"\|"attention"\|"unknown",connectivity:"ready"\|"synchronizing"\|"offline"\|"setup_required"\|"unavailable",capabilities:Capabilities,historyCoverage:HistoryCoverage,reason?:Text}` |
| UnboundRow | `{rowId:UUID,serverId:UUID,workspaceId:Id,terminalId:Id,paneId:Id,provider:"omp"\|"codex",title:Text,state:"setup_required"\|"ambiguous"\|"unavailable",reason:Text}`; connection-local row identity, **no** fabricated sessionId/generation/nativeId and no mutation target |
| NativeAgentStatus | `"idle"\|"working"\|"blocked"\|"done"\|"unknown"`; native desktop display only, not provider busy/capabilities/attention |
| NativeWorkspace | `{workspaceId:Id,number:Uint,label?:Label,focused:boolean,activeTabId?:Id,agentStatus:NativeAgentStatus,displayUnavailable?:"too_large"\|"unsafe"}` |
| NativeTab | `{tabId:Id,workspaceId:Id,number:Uint,label?:Label,focused:boolean,agentStatus:NativeAgentStatus,displayUnavailable?:"too_large"\|"unsafe"}` |
| NativePane | `{paneId:Id,terminalId:Id,workspaceId:Id,tabId:Id,label?:Label,title?:Label,terminalTitle?:Label,displayAgent?:Label,agent?:Label,agentName?:Label,focused:boolean,agentStatus:NativeAgentStatus,restoreReason?:"restore_failed",displayUnavailable?:"too_large"\|"unsafe",target?:Target,unboundRowId?:UUID}`; native labels/agent alias are separate from provider conversation title; no shell mutation target |
| NativeHierarchy | `{serverId:UUID,serverIncarnation:UUID,freshness:"current"\|"stale"\|"unavailable",reason?:"source_unavailable"\|"too_large",workspaces:[NativeWorkspace],tabs:[NativeTab],panes:[NativePane]}`; proven server/incarnation scope, flat returned parents/order, not another mutable tree |
| SessionState | `{summary:SessionSummary,nativeTurnId?:Id,desktopSelection:"unknown"\|"bound_launch_thread",activity:Activity}` |
| Activity | `{state:"idle"\|"working"\|"unknown",text?:Content,toolRecordId?:Id}`; transient, not a completed history record |
| Content | `{kind:"text",text:Text}` OR `{kind:"ref",handle:UUID,byteLength:Uint,mime:"text/plain"\|"application/json"}` OR `{kind:"unavailable",reason:"source_truncated"\|"not_cached"\|"unsupported"\|"too_large",byteLength?:Uint}` |
| HistoryRecord | `{id:Id,parentId?:Id,turnId?:Id,order:Uint,timestamp?:Uint,role:"user"\|"assistant"\|"tool"\|"system"\|"unknown",kind:"message"\|"tool"\|"notice"\|"unsupported",text?:Content,tool?:Tool,original?:Content,source:"persisted"\|"live"\|"native_page",complete:boolean,sourceTruncated:boolean}` |
| Tool | `{name:Text,callId?:Id,status:"running"\|"succeeded"\|"failed"\|"unknown",input?:Content,output?:Content}`; no terminal escape execution |
| PendingRequest | `{requestKey:UUID,provider:"omp"\|"codex",kind:Presentation.kind,presentation:Presentation}`; bound internally to exact native request ID/type and Target; no generic responseSchema interpreter |
| Error | `{code:ErrorCode,message:Text}`; fixed safe message, no provider payload/token/path leakage |

`order` is reducer-assigned stable order within the current generation, not a fabricated native ID or durable upstream cursor. Preserve native IDs/parent links; unknown native history entries retain an `unsupported` record and safe original content, not a guessed assistant message. OMP records use file order then native live append order, displaying parent/branch cues; Codex orders turns/items using native ordering, not phone arrival time.

Finite `Presentation` / `answer` pairs (provider source-specific projection is in [OMP](OMP_BACKEND.md) and [Codex](CODEX_BACKEND.md#native-approvals-and-questions)):
- `omp_select`: `{kind,title:Content,options:[{value:Text,label:Text,description?:Content}],selectionMarker:"radio"|"checkbox",helpText?:Content,markableCount?:Uint,selected?:Uint,checked:[Uint]}` → `{kind:"omp_select",action:"submit",value:Text}` or `{kind:"omp_select",action:"cancel"}`. Selection must be an offered value; checkbox cards return one offered string (sequential native toggle), not multiselect answers. Markable count is at most the option count; trailing control rows have no marker. Preserve ordered duplicate options/checked indices. Native omitted marker projects radio; unrepresentable native indices/counts make the entire card desktop-only.
- `omp_editor`: `{kind,title:Content,prefill:Content}` → `{kind:"omp_editor",action:"submit",text:Text}` or `{kind:"omp_editor",action:"cancel"}`.
- `codex_approval`: `{kind,approvalType:"command"|"file"|"permissions",title:Content,details:Content,choices:[{choiceId:Id,label:Text,scope:"once"|"turn"|"session"|"policy",details?:Content}]}` → `{kind:"codex_approval",choiceId:Id}`. Host stores exact offered native results in memory; phone cannot supply/amend grant JSON.
- `codex_questions`: `{kind,isBlocking:boolean,questions:[{id:Id,header:Text,prompt:Content,isOther:boolean,isSecret:boolean,options:[{label:Text,description:Content}]}]}` → `{kind:"codex_questions",answers:[{id:Id,values:[Text]}]}`; unique exact question IDs, one answer per question, offered labels unless free input is native-supported. No fabricated cancel API.
- `codex_form`: `{kind,title:Content,fields:[FormField]}` → `{kind:"codex_form",action:"accept",values:[{id:Id,value:FieldValue}]}` or `{kind:"codex_form",action:"decline"|"cancel"}`. `FormField={id,title:Text,description?:Content,required:boolean,type:"string"|"number"|"integer"|"boolean"|"single"|"multi",default?:FieldValue,format?:"email"|"uri"|"date"|"date-time",minLength?:Uint,maxLength?:Uint,minimum?:decimalString,maximum?:decimalString,minItems?:Uint,maxItems?:Uint,options?:[{value:Text,label:Text}]}`. Only constraints meaningful to the declared type are allowed. `FieldValue` is a string (including exact decimal for numeric types), boolean, or array of strings for multi. Validate required/unique IDs, format, scalar length, range, membership and cardinality on both sides.
- `codex_url`: `{kind,title:Content,url:Text}` → `{kind:"codex_url",action:"accept"|"decline"|"cancel"}`. Opening a URL is separate from answering and does not close the card.
- `desktop_only`: `{kind,title:Content,reason:Text}`; no answer accepted. Unsupported/oversized/secret-bearing raw request data is never hidden behind an actionable partial form.

Preserve finite-form array order and repeated values/options/checked markers
unless the pinned native contract explicitly requires uniqueness.
Unique question/field/answer correlation IDs do not imply `uniqueItems`;
do not deduplicate multi defaults or answers or invent single-answer bounds.

String-format validation is shared Rust/Kotlin syntax validation, not
deliverability, navigation or future-event verification:
- `date`: Gregorian `full-date` in [RFC3339 §5.6](https://www.rfc-editor.org/rfc/rfc3339.html#section-5.6), four-digit year0000..9999 with actual month/day/leap-year bounds.
- `date-time`: [RFC3339 §§5.6–5.7](https://www.rfc-editor.org/rfc/rfc3339.html#section-5.6), explicit `Z` or numeric offset, optional fractional seconds and permitted lowercase `t`/`z`. Seconds60 require the UTC end-of-month23:59 location after applying the offset; no online leap-second schedule lookup or claim that a future insertion is scheduled.
- `uri`: [RFC3986 §3 `URI`](https://www.rfc-editor.org/rfc/rfc3986.html#section-3), required scheme with optional fragment, any scheme, valid percent escapes/authority/path syntax, no relative reference. This is not the fragment-free `absolute-URI` production or the separate HTTP(S)-only explicit-navigation policy.
- `email`: ASCII `Mailbox` grammar in [RFC5321 §§4.1.2–4.1.3](https://www.rfc-editor.org/rfc/rfc5321.html#section-4.1.2), including quoted local parts and valid address literals. No multiple-address/header syntax, international-email claim or DNS/SMTP lookup.

Use the same boundary/edge vectors in both consumers. Do not substitute
browser input heuristics or a platform parser's narrower offset/date range.

## Methods and exact params/results

`bootId:UUID` is additionally REQUIRED in every row marked B. `target` is a current `Target`; no raw provider IDs, executable paths, Herdr methods or shell commands are accepted. Request UUID is action ID for rows marked A. All operations except pair.confirm require paired identity; hello must complete before other authenticated operations.

| Method | B/A | Params beyond bootId | Exact result |
|---|---|---|---|
| pair.confirm | – | `{token:Text,deviceName:Text}` | `{hostId:<EndpointId>,deviceId:UUID,paired:true}` |
| hello | – | `{appVersion:Text,deviceName:Text}` | `{hostId:<EndpointId>,deviceId:UUID,bootId:UUID,protocol:2,backends:[Backend]}` |
| host.snapshot | – | `{}` | `{subscription:UUID,seq:0,servers:[Server],projects:[Project],sessions:[SessionSummary],unbound:[UnboundRow],hierarchy:[NativeHierarchy]}`; atomically replaces this connection's previous host subscription |
| session.open | – | `{target}` | `{subscription:UUID,seq:0,state:SessionState,pendingRequests:[PendingRequest],history:[HistoryRecord]}` |
| session.history | – | `{target,viewId?:UUID,cursor?:Id,limit?:Uint}` | `{viewId:UUID,records:[HistoryRecord],nextCursor?:Id}`; native head when both viewId/cursor omitted; otherwise both required; limit1..100 |
| terminal.read | – | `{target}` | `{target,text:Text,truncated:boolean}`; visible native text only, at most16KiB UTF-8 |
| session.unsubscribe | – | `{subscription:UUID}` | `{unsubscribed:true}`; accepts host/session subscription, repeated removal succeeds, never stops agent |
| session.create | B/A | `{serverId:UUID,projectId:UUID,provider:"omp"\|"codex",name:Text}` | `{receipt:Receipt}` including reserved/running creation on timeout |
| workspace.rename | B/A | `{target,label:Label}` | `{receipt:Receipt}`; exact-Target workspace only |
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

Read requests may retry with fresh request IDs, including the same still-valid history cursor. Read/ordinary non-agent operation deadline is15 seconds; provider-write boundary timeout is10 seconds; create waits at most120 seconds, then returns its current receipt while the existing job continues. Native bounded operations (e.g. Herdr's30-second launch) record intent before start and are polled without holding the transport reader. Newly reserved session creation returns its actual committed Reserved receipt after original-job registration; canonical progress follows through events/action.status while that same job continues. Client timeout after reservation is **not** failure/cancellation. No second response is sent; later results arrive via events/action.status. Invalid/expired cursor or handle gives its explicit error. Limit failure never silently clips inventory or an actionable form.

ErrorCode is exactly: `invalid_request`, `unsupported_version`, `unsupported_method`, `unauthorized`, `pairing_failed`, `stale_host`, `stale_session`, `not_found`, `not_ready`, `invalid_answer`, `already_resolved`, `action_id_conflict`, `action_capacity`, `content_expired`, `cursor_expired`, `busy`, `resource_limit`, `rate_limited`, `timeout`, `backend_error`, `push_rejected`, `stale_registration`. `not_found` covers unknown authorized server/project/session; `backend_error` covers a known native rejection only, not unknown dispatch outcome. Post-reservation errors are recorded in Receipt rather than returned as bare errors.

## Bounded terminal reads

`terminalRead` defaults to false when absent. True requires a current verified
OMP/Codex binding with Herdr’s atomic read identity; cached inventory and
transport connectivity cannot grant it. `terminal.read` is an authenticated
read under the ordinary15-second limit, with no action receipt, input, scrolling,
history pagination, durable content handle or automatic polling.

Capture the original Target, provider source epoch and host-local native read
identity. Check authorization and that same live context before dispatch and
again atomically with response enqueue after native I/O. A late response cannot
be admitted after source replacement, revocation or Target loss. Native epoch
values retain their exact u64 representation on the host and never cross the
phone wire. Enforce the16KiB text bound and actual encoded1MiB frame bound.
Terminal output stays memory-only; no cache, logging, clipboard or screenshots
of sensitive content. It is separate from redacted provider history.

This unreleased-v2 capability addition requires the updated APK before companion
enablement: older APK event decoders reject the new nested field. Hello has no
feature negotiation; do not infer support from its current appVersion string.
New clients accept old summaries with false capability. Concurrent old-client
support and APK downgrade after new summaries are stored are not established.

## Ordinary-shell consumers (planned)

Separate resource kind, same State/storage/security/receipt owners. A
`ShellTarget {shellId,generation}` cannot resolve through provider Target,
SessionSummary, history or SessionOpen. Generalize the persisted identity key
as Agent(existing key) or Shell(host/server/incarnation/terminal), preserving
all existing agent UUIDs, generations, displays, receipts and obligations in an
idempotent transaction. Both kinds share the existing512-resource limit.

HostSnapshotResult adds default-empty `shells`; NativePane may associate exactly
one provider Target, unbound row or shellTarget. Shell summaries include exact
parent/display identities, connectivity and independent read/create/input/close
capabilities; input/close remain false. Positive native shell scope, permission
epoch and read identity stay host-local. Changed proof/source replaces generation;
cached records never restore authority. No terminal text is persisted.

`shell.read {target}` returns target/text/truncated under the existing16KiB read
boundary. Revalidate the same original source, native scope, socket evidence and
paired authorization before I/O and atomic publication. `shell.create
{bootId,target}` uses the existing action UUID, fingerprint, security boundary,
non-clone entered DispatchTicket/checkpoint and receipt settlement. No new
list/open RPC, queue, endpoint, discovery engine or native method is introduced.

ShellCreate receipts require shellTarget and forbid provider target/creation.
Their `shell_creation` result retains monotone optional server/workspace/tab/
pane/terminal/createdTarget facts; success requires the complete native-created
identity set. Correlate native operation/scope and retain validated facts before
awaiting their existing durable checkpoint. Cancellation/revocation/lost or
malformed acknowledgement preserves known facts and settles Uncertain unless
correlated definitive no-effect evidence exists. Reconciliation never resends.
IDs alone never authorize adopting an uncertain resource or rewriting a terminal
receipt. Opening a created terminal requires its exact current inventory Target.

Native creation remains under its existing five-second budget and
[bound shell contract](HERDR_INTEGRATION.md#bound-ordinary-shell-foundation-planned).
Ship the matching APK before companion enablement: strict old clients do not
negotiate these new fields. Schema and both language consumers cut over together.

### Planned Text/Keys input

`shell.input {bootId,target:ShellTarget,input}` returns the existing receipt.
Use the canonical Text/Keys grammar matching native `shell.input_bound`, including
required modifier booleans and exactly one Unicode scalar for Character. No
adapter key enum, normalization, implicit Enter or draft-to-key conversion.
ActionMethod ShellInput requires shellTarget and forbids provider target,
creation metadata and creation resources. Input capability comes only from the
positive native shell grant; close remains false.

The existing reservation, boundary, same entered ticket and settlement owner
retain typed ShellInput progress before any checkpoint await. Optional
encodedBytes/writtenBytes/flushCompleted contain no input; total is bounded at
16396, immutable when known, written never regresses or exceeds it, and flush
true never regresses. A zero-total native refusal does not establish the total;
zero written alone is not no-effect proof. Correlate operation/original scope.
Normal fenced Accepted plus complete write/flush and the original acknowledgement
validator permits Succeeded (input accepted, never command completion).
Definitive correlated pre-effect refusal permits Failed; partial, lost, changed
or unproved acknowledgement remains Uncertain with known facts retained.
Ignore only unrelated topology revision/readiness after acknowledgement, keeping
all original physical/source/permission/Target fences. No follower read or retry.

Reuse existing action storage with ordinaryComposer=false. Do not persist input
or command recovery text on either device. Original viewer-context admission,
content-free receipt recovery and explicit separate-action consent apply.
Ship the matching APK before enabling input; no database migration is added.

## Bound Worktrees browsing

Planned coordinated capability; not yet available in the installed app.
`worktreesRead` is optional and defaults false. It requires the original verified
OMP/Codex binding's pinned read identity and explicit native snapshot support;
terminal-read support, transport state and version strings cannot grant it.

Authenticated `worktrees.list {target}` returns `{target,worktrees}`. Each entry
has `path`, `label`, optional `branch`/`head`, booleans `isBare`, `isDetached`,
`isLocked`, `isPrunable`, and optional `isLinkedWorktree` when native evidence
establishes it. No repository source metadata or open-workspace identity is
inferred. Returned paths are display data, never file or mutation authority.

Reject more than128 entries, paths over4096 UTF-8 bytes, or labels/branch/head
fields over1024 bytes. Require valid display strings without control characters
and enforce the actual encoded1MiB frame bound independently. Reject the whole
result on malformed/oversize output; never return silent partial success.

Use the ordinary15-second read budget, original Target/source/native identity,
authorization before dispatch, and the same state lock for final context check
and response enqueue. One native bound-list RPC, no supplementary lookup, receipt,
automatic retry or polling. Keep native epochs and provenance host-local. Native
scope mismatch maps to StaleSession; unsupported/generic unavailable to NotReady.
Map repository absence, resource limit and timeout only from native codes that
actually distinguish them. Apply the updated-APK-before-enablement rule above.

## Bound Changes browsing

Native `changes.read_bound` has the same original pane/terminal/read-identity
parameters and echoed scope as Worktrees. Its separate optional-false snapshot
flag is `changes_read_bound`; Goatr capability `changesRead` also defaults false.
Authenticated `changes.read {target}` returns `{target,changes}`:

```text
changes {branch?, head?, detached, entries: [ChangeEntry],
  stagedPatch, unstagedPatch}
ChangeEntry = {kind:"tracked", path, originalPath?, index, worktree,
  submodule?: {commitChanged, trackedDirty, untrackedDirty},
  stagedBinary, unstagedBinary} | {kind:"untracked", path}
```

Index/worktree states are `unchanged`, `added`, `modified`, `deleted`, `renamed`,
`copied`, `type_changed` and `unmerged`. Reject unknown/cross-variant fields.
Tracked entries require both status fields and binary flags; originalPath is
required exactly when either status is renamed/copied. Submodule flags require a
native submodule record. Untracked entries contain only kind/path, without
invented tracked states or binary classification. Binary flags mean that side’s
observed numstat reported binary; false does not prove textual content. If status
and numstat cannot be joined uniquely, reject the whole observation rather than
guessing paths or dropping records. Detached HEAD has no branch;
an unborn branch has no head. Paths remain display data.

Planned repository context extends this result with optional `repositoryContext`:
`{unborn, upstream?:{name,ahead?,behind?}, latestCommit?:{oid,subject}}`.
Missing context means older native support, never an inferred unborn state.
Present unborn context requires a named non-detached branch, no head or latest
commit; otherwise latestCommit.oid equals the full lowercase40/64-hex head.
Detached forbids branch/upstream. Upstream counts are a canonical nonnegative
bounded Uint pair or both absent (comparison unavailable, never invented zero).
Upstream name and commit subject are strict UTF-8, at most1024bytes and no
controls; an empty subject is valid. Metadata is secure display data only.

Observe context from existing guarded porcelain-v2 status with explicit
`--ahead-behind`, rejecting duplicate/malformed headers. Read the latest commit
only by validated captured full OID through the same guarded runner and fixed
arguments, with no notes, signatures, replace objects, lazy fetch or external
helpers. Repeat the same root status and compare branch/head/unborn/upstream/
counts before final original-source validation; mismatch fails without retry.
All observations share the existing5-second/256KiB aggregate native budget and
execution/cleanup fences. Counts reflect local refs, with no network fetch;
these bracketed observations do not claim an atomic Git/worktree snapshot.
Android uses the existing secure Changes surface and original refresh ticket,
including clean trees. No new method, receipt, storage or mutation is added.

Reuse Worktrees path/branch/head/count limits, original-context authorization,
15-second read budget and same-lock final State validation/enqueue. A dedicated
`PatchText` permits at most256KiB UTF-8 per patch field; generic `Text` stays
unchanged. Permit LF, CR and TAB, reject other controls and invalid encoding.
The native aggregate256KiB limit and actual encoded1MiB frame cap both apply.
Oversize/malformed results fail wholly, without truncation, paging or ContentRefs.
Keep native provenance host-local; no receipts, automatic polling or retries.
Explicit native mismatch/absence/limit/timeout codes use the Worktrees mapping.

## Bound workspace rename

`workspaceRename` is optional and false when absent. `workspace.rename` uses the
existing bootId, action-envelope UUID, authorization, reservation and receipt
semantics. Label uses the existing nonempty Label bound: at most512 UTF-8 bytes
and128 Unicode scalar values, with no control characters or normalization.
Resolve the workspace only through the original Target/source and atomically
validate its native binding at the actual rename boundary.

A correlated native success returns `workspace_renamed` for the original Target
and workspace. Entered/resulting labels are not stored in receipt metadata or
logs; authenticated hierarchy supplies the observed label. Later context loss
cannot convert an acknowledged effect into a false no-effect failure. Unknown
writes remain uncertain and never replay. No new provider, readiness, permission
or generation exception is introduced.

## Identity, ordering and events

[Herdr](HERDR_INTEGRATION.md#durable-identity-and-routing) owns the persistent mapping. Stable sessionId names a proven desktop/native binding; reconnect keeps it while replacing generation. A different native conversation/terminal/owner creates a different sessionId; room replacement of the same OMP process/conversation keeps sessionId but replaces generation. Pane/workspace routing changes alone do neither. Every companion boot has a new bootId and every restored session receives a fresh generation. All old provider handles, content/cursors and pending requests become invalid.

One serialized reducer per session, plus one host/notification reducer and one ordered writer per connection. While a request dispatch waits, keep its original deadline and the single guarded output step concurrently polled; do not dequeue another request. Finish an in-progress frame before queuing the dispatch response, preserving FIFO and authorization checks. A closed queue or failed write closes the connection; never resume after canceling a partial frame. Snapshot capture, seq0 response enqueue and subscriber activation occur atomically in the owning reducer. Events following that snapshot start at 1. Phones never receive the queued event before its response. Opening a phone subscribes to an existing reducer, not a new native provider subscription. OMP welcome/final snapshot and Codex resume-response/queued-notification boundaries install first; queue subsequent native events until installation completes.

Capture the local source epoch when starting a snapshot installation; both
completion and abort must match that installation's Target and epoch atomically.
A delayed callback from an aborted capture cannot consume a newer installation
for the same Target. Phone history-page completion and native-cursor registration
likewise compare the captured source epoch under the reducer lock; Target equality
alone does not authorize merging or attaching a page after source replacement.

An OMP periodic-observation deadline is a source-local Synchronizing gate. It
retains authenticated native reads, history and dialogs but rejects mutations;
only the adapter's complete fresh same-binding observation can clear it.
Topology revalidation or ordinary provider state updates cannot clear that gate.

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

Native hierarchy is a required clean unreleased-v2 cutover in Rust/Kotlin and
both strict/tolerant Response schema definitions, not an optional legacy alias.
Graph IDs/parents/array order and workspace/tab numbers come from native
Snapshot, independent of provider/project filters. Validate unique identities,
returned parents/active-tab references and exact supported pane associations.
Current graphs omit reason; stale graphs require source_unavailable and strip
Target/unbound associations; unavailable graphs require a reason and empty
arrays. Missing workspace/tab labels require displayUnavailable; unsupported or
oversized display metadata cannot silently become a fabricated label/status.
Optional pane display fields remain unknown when not supplied. Restore reasons
are coarse and contain no raw paths/reports. Native graph freshness is separate
from provider readiness and `desktopSelection`.

Only a current proven incarnation/terminal/native binding may supply a pane's
Target or supported unbound row, never both. Offline summaries and stable Goatr
session/cache aliases are historical rows, not native parents; never join them
to a graph by matching strings across incarnations. Native metadata/focus/route
refresh preserves Target/generation/source epoch/dialog/content. Actual binding
replacement removes the association and applies existing source invalidation.
Desktop focus never opens/retargets a phone conversation.

The producer marks complete records unavailable/too_large if the global node
budget is exhausted **and the source is confirmed current**; otherwise the
empty record remains unavailable/source_unavailable. Native dirty transitions
and the atomic Source commit fence apply to overflow records too, independently
of supported sessions. A reduced graph never supplies freshness proof.
The existing snapshot encoder validates relationships and associations first,
then only an observed transmitted-byte/member overflow may replace whole graph
records with unavailable empty arrays, in reverse server publication order,
preserving complete other records where possible. It preserves source_unavailable
for stale graphs; only a current source becomes display-only too_large. Base
supported inventory overflow still fails resource_limit; malformed/source errors
are not budget fallbacks. Frames/depth/member and global64/512 caps are unchanged.

The existing
`hierarchy_budget_preserves_unavailable_source_when_collapsing_stale_graph`
regression and shared Kotlin overflow fixture retain these semantics;
after-repair checks require coordinator validation.


## Mutation receipts

Canonical request hashing: hash UTF-8 canonical JSON of `{method,params}` including bootId/target, excluding envelope id/version. Sort object keys by Unicode scalar order, preserve array order; no whitespace; escape only quote, backslash and U+0000..001F (lowercase `\u00xx`); emit other scalars literally; integers in decimal without sign/leading zero unless negative is schema-allowed (none in v2). No normalization of Unicode or strings. Normalize schema defaults before hashing (only documented omitted limit defaults; action params have none). Structured noninteger numbers are forbidden, so Rust/Kotlin need no floating-point canonicalization; ordinary `serde_json` serialization is not the protocol canonicalizer. Fixtures contain input bytes, canonical bytes and SHA-256 hex and must match across both consumers. Durable companion fingerprints use HMAC-SHA256 over canonical bytes with a private local receipt key; phone uses its own protected key. This prevents offline guessing of secret answers from stored unkeyed hashes. Do not persist canonical bytes.

Check order: (1) frame/schema/limits; (2) authenticated peer/hello and current authorization epoch; (3) peer-scoped action ID lookup and fingerprint comparison; matching existing receipt is returned **before** stale boot/target checks, with no native dispatch; conflict errors; (4) for an unseen ID validate current boot, target/capability/turn/request and answer; (5) reserve durably under the peer fence; (6) recheck authorization and target at every actual side-effect boundary. Revoked peers cannot use receipt lookup to read anything. `action.status` can retrieve an old-boot receipt after a new hello; unknown/expired status never authorizes resend. A matching receipt query is not a native retry.

`reserved` is authorized **no-effect intent**, not native readiness or permission
to write. OMP may carry an already-Ready original through a late dirty-topology
revision into its one reservation, only for the same Target and captured source
epoch, a current physically proven connection, noninstalling/nonfailed Core and
valid capabilities/request witnesses. The canonical Core rule itself restricts
this Synchronizing exception to OMP and `topology_dirty`; it does not enable Codex
or clean/Offline/replaced sources. The original job retains its absolute deadline
and must regain authoritative Ready and pass the unchanged strict authorization/
target/native-source validator before any effect. There is no replay, reservation
retry, existing-receipt rewrite or wait inside the SQL/authorization fence.

Source readiness may also change after that original's durable dispatch intent
but before any native effect starts. OMP may retain the same unstarted claim
while awaiting only its proven, physically fenced pending revision: the receipt
is then truthfully `dispatching`, not `reserved` or a successful native write.
No SQL/authorization/writer permit is held during that readiness wait; no second
intent or dispatch ticket is created. Strict authorization and exact-source Ready
validation still precede the single actual start. Terminal unstarted failures have
definitive no-effect proof; a process crash still leaves honest `uncertain`
recovery, never replay. The original pending deadline and ten-second started-I/O
limit are distinct. Already-failed receipts are never resumed by this mechanism.

`Receipt={actionId:UUID,method:ActionMethod,bootId:UUID,target?:Target,shellTarget?:ShellTarget,createdAt:Uint,updatedAt:Uint,state:"reserved"|"dispatching"|"dispatched"|"succeeded"|"failed"|"uncertain",error?:Error,result?:ActionResult,creation?:CreationProgress}`. No text, answer, original request JSON or grant payload is stored in receipt metadata on either device. Internal records also contain peer ID, authorization epoch and keyed fingerprint, never returned on wire.

Allowed transitions: new→reserved→dispatching→dispatched or succeeded; reserved→failed before any effect; dispatching→failed only with definitive native rejection/no-effect proof for the attempted operation; reserved/dispatching→uncertain after crash/lost outcome; dispatched→succeeded only upon correlated native acknowledgement; uncertain→succeeded/failed only with definitive reconciliation proof, never replay. For multistage creation, dispatching persists across stages: a known later-stage failure can be failed **with all earlier confirmed resources retained**; any unaccounted stage effect instead makes the whole outcome uncertain. Terminal failed/succeeded do not restart. Dispatched is an honest no-ack receipt, not queued retry. Persist dispatching before attempting native write: crash between commit and write is indistinguishable from execution and becomes uncertain. Restart marks unfinished reserved/dispatching actions uncertain, never resumes them automatically.

`ActionResult` is `{kind:"workspace_renamed",target:Target,workspaceId:Id}` or `{kind:"prompt",nativeTurnId?:Id}` or `{kind:"cancel",nativeTurnId?:Id}` or `{kind:"response",resolution:"written"|"acknowledged"|"resolved_elsewhere_or_unknown"}` or `{kind:"created",target:Target,workspaceId:Id,terminalId:Id,paneId:Id,nativeId:Id,bindingId?:UUID}`. OMP writes ordinarily end dispatched; dialog closure alone does not establish the winner. Codex clientUserMessageId correlates but does not guarantee native idempotence. Succeeded means admission/operation acknowledgement, not entire turn completion.

### Ordered creation progress

`CreationProgress={serverId:UUID,projectId:UUID,provider:"omp"|"codex",stage:"workspace"|"owner"|"thread"|"relay"|"launch"|"ready",stageState:"not_started"|"dispatching"|"confirmed"|"uncertain"|"failed",resources:CreationResources}`. `CreationResources={workspaceId?:Id,paneId?:Id,terminalId?:Id,nativeId?:Id,bindingId?:UUID,sessionId?:UUID,generation?:UUID,ownerStarted?:boolean,relayStarted?:boolean}`; omit unknown values rather than report false certainty. Host-only metadata retains socket/process epochs, unit names and per-stage intent/results from the backend contract. Persist stage intent and returned IDs before the next side effect. OMP skips owner/thread and confirms nativeId after launch; Codex follows its ordered steps. Ready/confirmed plus resulting Target is required for create succeeded.

Creation deadline returns `{receipt}` in its actual running state, not false success. Disconnect does not cancel the durable in-process job; companion restart does not replay it. A failed later stage retains earlier resources and reports partial creation; any unconfirmed effect makes the receipt uncertain. Record correlation alias/actionId even if workspace/thread IDs were lost, expose the known resources, inspect read-only native state and require explicit operator reconciliation when proof is insufficient. Never delete the workspace, restart a launch, create another thread, or roll back already-started desktop work silently. Revocation stops subsequent stages; confirmed earlier resources survive.

A correlated Herdr `agent.start` response proves launch submission, not native
conversation readiness. Settle that bounded write with launch/dispatching and
its known resources/ack metadata; do not invent nativeId or confirm Launch.
The original job reads native identity outside the write/start fence, then
confirms Launch only with the required correlated proof. Dispatching grants
no next-stage permission. Revoke or crash while that proof is pending preserves
known resources as uncertain, without replay or killing admitted desktop work.
For workspace creation, persist the exact returned workspace/root-pane/terminal
IDs before separately confirming them through the authoritative snapshot;
readiness/confirmation waits do not hold the native write boundary.

Codex Owner/Relay synchronous service admission may likewise settle the existing
entered stage as Dispatching while the original job observes native readiness
outside the bounded fence. This is valid only for Codex Owner/Relay, not Thread
or Ready and not a broader OMP change. Workspace and confirmed-probe stage
prerequisites remain; known resources are monotone, nativeId cannot change in an
intermediate result, and sessionId/generation remain absent. A known bindingId
or observed start fact does not confirm readiness or permit the next stage.
Only a later correlated Confirmed result admits advancement. The original
admission ticket must settle before revoke/drain acknowledges; already admitted
independent resources survive, with uncertainty retained and no effect replay.
For both revoke orderings—an entered ticket settling after removal, or a settled
admission whose ticket is already complete—persist the receipt, progress and
last stage record as Uncertain while retaining exact known resources, original
nativeId/bindingId and admission metadata/proof. The shared finite classifier
also covers the unchanged Launch and Workspace/ShellProbe submission cases;
it never turns these admissions into no-effect failure, Ready or another launch.

Host-local Workspace preflight uses one distinct `WorkspaceOperation::ShellProbe`
intent/ticket/ACK/result subrecord after the real Workspace/Create record.
It adds no wire stage or remote method. Exact action/new-root/shell/process and
private result-object metadata contain no raw nonce, command or result bytes.
Probe enqueue ACK retains Workspace/Dispatching; it is not shell execution or
launch permission. Only a matched complete result plus current kernel/source/
focus/idle/path proof confirms the subrecord. The finite GNU Bash5.3p9
(bash/bash-interactive) and zsh5.9/5.9.1 source/version gate does not replace
that actual Herdr-configured new-root shell proof. Every later provider-stage
intent requires it. Tickets bind operation/attempt, so delayed workspace/probe results
cannot settle each other. Restart/revoke preserve admitted unknown effects as
uncertain and never replay the query. Known rejected resolution can fail while
retaining all earlier resources. See the narrow [Herdr probe](HERDR_INTEGRATION.md#once-only-new-root-shell-probe)
and its non-atomic foreground/queue-recall limits.

`Security::native_guard` is the existing paired-epoch/revoke guard for accepted
original native jobs; `guard` additionally requires the active current hello for
network reads/lookups/output. Original bounded action jobs survive disconnect/
replacement/client timeout, but neither authorization removal nor native source
replacement. Release the short native guard before provider I/O/readiness/full
turn waits. Every external effect still enters its actual connected-FD boundary.
`Security::reserve(auth,request,validate,committed)` invokes `committed(Receipt)`
only after a successful new `Storage::reserve` commit, inside the existing
serialized storage job before response delivery. Existing lookup and rejected
capacity/alias/SQL reservations bypass it. The original request and paired epoch
remain owned by that committed job; no second reservation/job owner is introduced.
OMP shutdown waits for serialized storage admission/no-op even at queue capacity
before taking its final job set, then joins owned originals before reporting a
barrier failure. Stopping rejects later validation; previously validated commits
register independently of their reply waiter, with no paired fence around the drain.

### Native browse views

session.open installs the latest live window, at most100 rows, with no history
cursor. session.history without viewId/cursor creates a native-head browse view;
continuations require both the returned viewId and opaque cursor. These reads
never insert older rows into the live reducer or share its order domain.
Backscroll follows the provider's opaque native successor beyond the live1000-row
window. A browse view makes no claim that the provider has unlimited retained
history: traversal ends only at that provider's terminal cursor, expiry or an
explicit capacity error. The shared browse owner preserves every per-page
successor when a captured page is split into smaller phone responses; it never
turns the local display/cache bound into a fabricated terminal cursor. New views
start at the head. Source expiry can return fewer.
HistoryRecord.order is meaningful only within its live subscription or browse
view. Rows on a page are ascending; following successors moves toward older
native positions. Do not compare ranks across views or with live arrival ranks.

Each materialized page freezes completed row membership, display values and its
successor. Incomplete native positions are consumed and excluded; later
completion appears in live state or a new browse view. This is per-page capture,
not whole-view MVCC. Repeating an explicit cursor with the same limit returns
the same IDs/order/nextCursor, even after live append or late completion.
Cursorless reads create new views and have no repeat guarantee.

One shared owner accounts live history, adapter history, captured browse rows,
positions and memoized responses under64MiB, deduplicating shared Arc objects.
At most64 browse tickets exist globally. Expiry is fixed15minutes from view
creation, never sliding. Capacity evicts the oldest other view, never clears all
healthy views; failure to reserve a page and successor returns resource_limit.
Fetches for one ticket coalesce. Native I/O holds no State mutex; source and
identity are revalidated before publication. Peer, Target, generation, source,
view, expiry or capacity loss returns cursor_expired for an existing ticket.

Each uncached advance performs at most one native history-page RPC, plus its
physical and complete loaded-membership guards. The whole Codex read has a14s
operation deadline inside the15s transport envelope, at most64 RPCs and16MiB
aggregate response bytes, with individual native frames capped at8MiB. Native
cursors remain opaque. Empty progress pages do not establish older coverage.
Cached responses retain identity checks but perform no native history re-read.
Content handles remain subject to their own expiry. Without an exact bounded
locator, return content_expired; do not scan history to rehydrate a cached page.
A new native view may project fresh Content handles from its exact source.

## Durable attention and read state

The two notification methods are the narrow addition required by live/push dedup, not a transcript sync framework. `AttentionEvent={eventId:UUID,sequence:Uint,sessionId:UUID,generation:UUID,category:"attention"|"completed",createdAt:Uint}`. Allocate and persist eventId/monotonic sequence per paired peer in the same transaction as the authoritative transition: new actionable request/known attention, or working→idle completion. Do not emit completion from disappearance, unknown busy state or initial/reconnected snapshot. Store native correlation where available. For stock OMP completion, persist a separate host-owned observed-cycle token/ordinal with the original Target/generation/private source epoch and connection: only nonreplayed live agent_start opens it and actual agent_end on that same continuously proven source closes it. Duplicate starts retain the cycle; repeated ends without an active cycle emit nothing. Initial snapshots, registry busy/input signals, turn_end, tool/history events, reconnect and restart never complete a cycle. Source gaps retire it without completion; intermediate idle/attention display checkpoints do not erase its independent start proof. This is observed live-cycle completion, not an invented native turn ID or history match. Event sequences never reset on companion restart or push-key rotation.

`ReadWatermark={sessionId:UUID,throughSequence:Uint}` is per peer/session, monotonic, advanced only through events that peer has received from authenticated sync/live state while actually viewing that session. Reject future/wrong-session sequence with invalid_request. `notifications.sync` returns ascending events afterSequence (default0), a complete bounded read-watermark list, latestSequence and first retained sequence (latest+1 if empty); max/default page100. `nextSequence` is last returned when more pages remain. `resetRequired` says requested history predates retention: refresh current state/read markers, do not invent missed notifications or replay an old backlog as new. Live and push share eventId; push alone neither marks read nor changes transcript. Probes are not AttentionEvents and have no session/sequence.

## Bounds, content and privacy retention

Android keeps one bounded display window. The first accepted browse page replaces
its live-ranked rows; subsequent pages share the same view and retain the server
nextCursor even after the local window evicts newer rows. Live updates may refresh
fields of already visible IDs, preserving their captured browse order; unknown
live IDs do not enter the browse window. Requests bind Target,
subscription and a client browse revision so stale responses cannot switch views.
Return to latest installs a fresh session snapshot and resets browse state.

This table is the single numeric authority for shared wire/storage/privacy limits. Security-specific admission and push retry timers are owned by their topics.

| Resource | Limit / eviction |
|---|---|
| Frame / JSON depth / aggregate members | 1 MiB / 32 / 16,384; reject before dispatch |
| Prompt / editor answer / total form answer | 256 KiB each; prompt/editor submit count decoded text UTF-8 bytes, other finite answers count complete canonical Answer JSON bytes including kind/action/IDs/arrays, not incidental wire whitespace; frame cap also applies; never truncate a send |
| Generic Id/cursor / label/device name / safe error | 512 bytes / 1–128 Unicode scalars and ≤512 bytes, no controls / 512 bytes |
| Form questions/fields/options | 64 questions/fields; 256 options per field and 1,024 total; too large → desktop_only |
| Inline Content / loaded content | 16 KiB inline; max16 MiB per payload, host aggregate64 MiB/session and256 MiB total LRU; TTL15 minutes or generation end |
| Content read | 1..65,536 bytes; offsets within byteLength, allow offset=end; base64 bytes may split UTF-8, client decodes after assembly |
| History page / inventory | ≤100 records/page within frame; ≤512 proven+unbound rows, 64 servers, 128 projects; capacity errors are visible, not silent truncation |
| Native hierarchy | ≤64 records, ≤512 total workspace+tab+pane nodes across graphs; complete records or explicit unavailable/too_large, never clipped/current-empty; shared frame/depth/member cap still applies |
| Pending requests / subscriptions | 64/session; 32/peer, 1 active host subscription/connection; capacity leaves excess native requests desktop-only |
| Outbound queue | 256 events or4 MiB, whichever first; overflow recovery above |
| Native backscroll | Provider-native cursor traversal with ≤100 records/page; Android retains one local ≤1000-record window and never suppresses a valid server successor |
| Browse tickets / lifetime | 64 globally; fixed15minutes; shared64MiB history accounting; explicit eviction/expiry |
| Host reducer history | 1,000 projected records/session, 64 MiB total; older records refetched from native history, not silently represented as complete |
| Receipts | 30 days from terminal/uncertain/dispatched update, 10,000/peer; only expired records evicted, action_capacity otherwise; unknown never permits replay |
| Attention events / dedup | 30 days or10,000/peer, oldest first; retain sequence high-water and read-watermarks through eviction; explicit resetRequired |
| Retired push generations | 30 days, max10,000/peer; evict expired only, reject rotation with resource_limit rather than discard unexpired replay protection |
| Android cached history/content | 50 MiB total, 1,000 records/session, 30-day age; evict oldest unviewed session then oldest records, content LRU; active visible content can be reloaded, never mislabel partial cache complete |
| Android cached native inventory | One stale serialized snapshot/paired host, shared50-MiB/30-day cache and Forget cascade; source must fit the wire cap, private stale-reason overhead permits at most1MiB+4KiB; oldest cached inventories evict after unviewed sessions but before viewed history/content; no live reducer/subscription authority or cached Target/unbound associations |
| Android recovery drafts | ≤20 drafts and1 MiB total, 7 days; bound to original host/session/generation/action; warn before replacing/expiring recovery, never auto-send |
| Android explicit local snippets | Planned:32/paired host,128 total; title ≤80 code points/320 bytes and body ≤8192 UTF-8 bytes; user-authored, explicit Save, reject capacity without eviction; delete/Forget cascade; no automatic history/form/draft import or native authority |
| Android unsubmitted drafts | rotation memory only; no durable general draft feature |
| Stored pairs/read-watermarks | ≤32 pairs, ≤512 session watermarks per peer; delete obsolete session watermark only after its last event expires |

Content handles are unguessable UUIDs authorized by current peer and Target, never pathnames; pinned while a chunk is being copied, released after completion/timeout. Eviction/TTL/generation/source replacement returns `content_expired`; reopen/refetch the item, not an old handle. Large fields over the per-payload ceiling show unavailable with source/limit explanation, never false full content. No automatic remote images. Opaque cursors are bound to target/source epoch and expire on replacement.

Redacted native display payloads belong to the host session/source, not the
phone that first expanded them. Producers can retain them before any phone
connects; every currently authorized phone with this host-wide authority may
read a valid reference for its Target. Revoking one phone must not expire a
shared reference still needed by another; actual peer authorization is checked
at the read boundary, never inferred from handle possession.

Apart from explicitly saved local snippets defined above, phone and companion retain only redacted display history, safe original history JSON and receipt metadata. Never persist pending-request originals, secret answers, capability links, tokens, push endpoints/keys in plaintext logs, or raw control frames. Sensitive native history fields are omitted/redacted before caching; secret answers remain memory-only even when uncertain. Recovery drafts apply to ordinary composer text only and survive process death after pre-send persistence; recording a local receipt must not erase them. Erase recovery on authoritative success/dispatch recorded and user-visible outcome; uncertain retains text, with explicit inspection/manual new action only, never auto-retarget or resend. Exclude all Goatr private state/cache/drafts from Android backup/transfer. Offline local forget deletes pairing keys, push registration/key, cache, drafts, receipts, dedup/read state and notifications for that host; it cannot revoke the server allowlist. Show desktop revoke instructions.

Disclose each ordinary recovery copy's seven-day deletion deadline before
pre-send persistence; the normal Send action may acknowledge that visible
retention notice, without a second send-confirmation dialog. Record this
action-bound acknowledgment so maintenance/process restart can enforce the
hard expiry without retaining expired plaintext while waiting for a screen.
Replacing other retained recovery copies needs a separate warning naming the
affected host/actions and explicit approval; no blanket future-eviction consent.

## Acceptance gates

[Testing](../ACCEPTANCE.md) maps every section to executable Rust/Kotlin fixtures, provider fault injection, instrumentation or owned runtime checks. No Goatr protocol test has been run during this planning revision.

## Planned bound agent alias rename

Authenticated `agent.rename {bootId,target,name:AgentName}` uses the existing
mutation reservation, boundary, receipt and no-replay owner. AgentName is the
required native alias grammar `[a-z][a-z0-9_-]{0,31}`; no clear-alias operation.
Separate capability `agentRename` defaults false. Only original Ready OMP/Codex
provider Targets qualify; ordinary shell authority never transfers to this method.
Native acknowledgement must echo exact original scope and requested alias.
`ActionResult::AgentRenamed {target,terminalId}` retains exact native effect
identity. Definite pre-invocation refusal is Failed/no-effect; lost, changed or
malformed post-invocation outcomes are Uncertain, never reconciled by another
read, replay or replacement selection. Preserve same-ticket facts and existing
post-effect original-source acknowledgement validation. No storage migration.

## Planned bound worktree removal

Bound inventory may supply an opaque candidateId for a positively proven linked
sibling, including closed/unmapped checkouts; managedWorkspaceId is optional
metadata. Neither displayed paths nor workspace IDs grant removal authority.
worktrees.remove_preview {target,candidateId} returns exact target/candidateId,
one-shot previewId, path, optional branch/head, removable and closed reason.
Receipt-backed worktrees.remove {bootId,target,candidateId,previewId} requires
positive worktreeRemove, original Ready source and unchanged paired authorization.
Result retains operation correlation and monotone effectStarted,
gitRemoveSucceeded, checkoutAbsent and registrationAbsent facts. Outcomes are
removed, refused_before_effect or uncertain; all post-start nonzero/ambiguous
results are uncertain. Preserve normal deduplication, generation fences and
no-replay semantics. Native object identities remain host-local.

## Planned authenticated attachment staging

Accepted next slice, unimplemented. Selection/upload creates private companion
files and offers path insertion; it never submits a prompt or changes native
provider permissions. Optional-false attachments requires explicit local
allow_attachments (default false), original Ready Target authorization, and a
retained proof that host staging and provider-visible paths identify the same
objects. Implement that proof before enabling the feature; no shell probe or
extra provider RPC. Missing proof means unavailable, not a passing placeholder.

Receipt-backed attachments.begin/chunk/complete/discard use bootId, original
Target and a fresh action UUID per gesture/chunk. Begin carries uploadId(UUID),
displayName/mediaType (each at most128 UTF-8 bytes), size and SHA256. Chunk carries
uploadId, exact next offset, canonical base64 of at most128KiB decoded bytes and
its digest. Complete/discard carry uploadId only. Read-only attachments.status
returns owner-bound state, committed offset, expiry and completed provider path,
never bytes. Result attachment progress retains uploadId/committedBytes and
known final-path facts; receipts/logs retain no payload, source URI or filename.
Redact data-bearing Debug/errors and preserve the1MiB frame bound.

Stage only UUID-named0700 directories/0600 non-executable files under
runtime_dir/attachments/<peerUUID>/<uploadUUID>. Phone filenames are display data,
never destination authority. Pin openat/O_NOFOLLOW root-relative handles,
dev/inode/owner/link-count1, and the original runtime_dir_agent namespace mapping
before publication. No project writes, symlink traversal or provider-policy bypass.
Limits include partials:8MiB/file,4 unfinished and32 retained uploads/peer,
64MiB/peer and256MiB/host. Reserve declared size before effect; unfinished expiry
1hour, completed expiry7days with no access-based extension. Cleanup/discard
deletes only exact retained owned objects, not replacements or arbitrary trees.
Keep payload-free consumed-upload tombstones through the existing30-day receipt
retention so old receipt/status identities cannot name a later object. Bound these
tombstones to4096 per peer; refuse new admission with ResourceLimit at the cap.

Serialize per-upload mutations through existing authorization/reservation/receipt
owners. Journal chunk intent before write; fsync before advancing committed
offset. Recovery may truncate only an uncommitted tail on the proven unpublished
owned object, never infer success from size. Complete verifies size/digest,
fsyncs, publishes exclusively and retains monotone object/path facts before waits.
Lost effects remain Uncertain until receipt/status proves them. Background/cancel
stops further admission, not already admitted jobs, and claims no remote deletion.
Explicit continuation uses confirmed offsets, unchanged original authority and
fresh action IDs; never resend uncertain chunks or restart automatically. A new
source/generation invalidates the old action lease; recovery metadata grants no
retargeting or insertion authority.

Attachment status and explicit discard are retained-upload ownership operations.
Require current boot/paired authorization and exact owning peer, uploadId and
stored original Target; they do not adopt a replacement Target/namespace or
require the old native source to become Ready again. Source/generation loss or
allow_attachments=false prevents new upload frames and insertion, while allowing
authorized owner status and exact-object cleanup. Revocation still denies peer
operations. This exception never relaxes native session mutation fences, and
uncertain/admitted jobs must settle before deletion.

## Agent process directory terminal creation (planned)

terminal.create {bootId,target:Target} is an explicit provider-Target action with optional-default-false capabilities.terminalCreate and ActionMethod TerminalCreate. TerminalCreateResult contains the existing durable receipt. Receipts require the original provider target and forbid shellTarget/creation metadata; their shell_creation result reuses monotone ShellCreationResources. Success needs all native server/workspace/tab/pane/terminal identities; createdTarget is an optional positively mapped ordinary ShellTarget, never allocated to adopt uncertain resources.

Use the original provider Connection Arc/process/source epoch/socket/native read identity through reservation, paired authorization, entered AgentShell checkpoint and settlement. Invoke shell.create_agent_bound once with the action UUID. Preserve all correlated facts before awaits, including changed/lost acknowledgement bodies. A definitive pre-effect refusal is Failed; partial, malformed, lost or post-effect scope loss is Uncertain. Reconciliation reads receipts and never replays. Existing ordinary shell.create and ShellTarget semantics remain unchanged.

## Secure artifact reads (planned)

`ArtifactSource` is a closed `session {target:Target}` or `shell {target:ShellTarget}` union tagged by `kind`. Optional `Capabilities.artifacts` and `ShellCapabilities.artifacts` default false. Artifacts are independent of terminal read/input/create and attachment grants.

Read-only methods are `artifacts.list {source,parent?:UUID,cursor?:UUID,limit:1..100}`, `artifacts.open {source,file:UUID}`, `artifacts.read {source,readId:UUID,offset,limit:1..65536}`, and `artifacts.close {source,readId:UUID}`. List returns source, bounded entries `{reference,name,kind:directory|file,size?}` and optional nextCursor. Open returns source, readId, name, size, SHA-256 and previewKind text|binary. Read returns source, readId, exact offset, canonical base64 data and eof. Close returns source, readId and closed. No action receipt, host path or native token crosses this wire.

Refs/cursors are peer/original-source/generation scoped, five-minute absolute memory leases. Names are control-free UTF-8 up to255 bytes, with ordinal labels for nonrepresentable native names. Listing is at most1000 entries/directory and depth16, with256KiB/source metadata, finite32source-authorities/peer128global and16directories/source64/peer256global. Metadata caps are4MiB/peer16MiBglobal; no active-authority eviction.

Files are at most32MiB. One active snapshot/read lease per authenticated peer is this implementation's stricter limit within the eight-handle contract ceiling, supporting one preview/export at a time. Byte reservations remain64MiB/peer256MiBglobal, with32native handles globally. Native immutable bytes expire ten minutes from native request admission. An uncertain host reservation additionally covers the bounded request startup margin until exact close or expiry; no premature quota release. Native open correlations are minted before dispatch; exact memory-close tombstones prevent canceled late opens from publishing.

Nothing persists artifact names, bytes, refs, cursors, read IDs or destination URIs to SQL/cache/receipts/notifications/recovery. Read payload Debug representations are redacted. Exact owner close is cleanup of a retained memory lease, not permission to read a changed source. New list/open/read always require the original current source.

## OMP enabled Skills and secure document reads

`skills.list` remains read-only and now identifies `provider` (absent means legacy Codex). OMP rows describe enabled public commands with scope `enabled`, exact `commandName` and optional opaque `documentRef`; the command must be `skill:` plus the row name. Their path field is forbidden and their errors list is empty: document paths stay exclusively inside the original extension. Codex rows require their exact native path and forbid these OMP fields and retain their exact-path mention grammar. Catalogue generation is the existing SessionSummary.skillsGeneration, never native process authority. Existing128-row/32-error/256KiB projection and encoded-frame bounds still apply.

Read-only `skills.preview.open {target,catalogueGeneration,documentRef}` returns those fields plus readId, size≤1048576 and lowercase SHA-256. `skills.preview.read {target,catalogueGeneration,readId,offset,limit}` returns matching target/generation/readId, exact offset, canonical base64 data≤65536 decoded bytes and eof. Limit is1..65536. `skills.preview.close {target,catalogueGeneration,readId}` closes only exact peer-owned memory and returns closed. No paths, provider process, native capability or action receipt is accepted for these methods.

Open reserves finite ownership before native dispatch. Rejected/late/unclaimed open results retain their original read identity for exact memory cleanup; no reopen/replay/replacement adoption. Reads require the original Ready Target/source/backend/native scope and catalogue; explicit catalogue refresh may advance only catalogue generation. Close may clean exact retained memory after read readiness loss. Complete size/SHA-256/strict UTF-8 verification precedes any phone document display; no partial prefix or content persistence.

Each admitted catalogue/open/chunk request reauthorizes its original native Skills scope after waiting for channel ownership and after the channel reply, before publication. Cached inventory alone cannot authorize transfer across a grant change. Original request I/O remains in the existing runtime job owner through matching-reply drain, even if its phone waiter disappears. A canceled queued waiter writes nothing; failed/ambiguous drain closes only that original channel. Rejected, canceled or unpublished preview results retain exact memory-cleanup ownership.
