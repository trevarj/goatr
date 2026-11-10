// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent native oracle. Never calls the adapter's history projector.
use super::*;
use futures_util::{SinkExt, StreamExt};
use goatr::codex_relay::{Manifest, OwnerEpoch};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

const RPC_BYTES: usize = 16 * 1024 * 1024;
struct ReadWork {
    calls: usize,
    bytes: usize,
    deadline: tokio::time::Instant,
}
impl ReadWork {
    fn new(deadline: tokio::time::Instant) -> Self {
        Self {
            calls: 0,
            bytes: 0,
            deadline,
        }
    }
    fn check(&self) -> io::Result<()> {
        if self.calls > 64 || self.bytes > RPC_BYTES || tokio::time::Instant::now() >= self.deadline
        {
            return Err(failure("independent native witness read budget exhausted"));
        }
        Ok(())
    }
}
pub(super) struct Witness {
    socket: WebSocketStream<tokio::net::UnixStream>,
    manifest_path: PathBuf,
    manifest_bytes: Vec<u8>,
    owner: goatr::config::CodexOwner,
    epoch: OwnerEpoch,
    herdr: ProcessEvidence,
    herdr_socket: PathBuf,
    herdr_object: Value,
    reads: usize,
    calls: usize,
    bytes: usize,
}
impl Witness {
    fn physical_observation(&self, p: &NativeProof) -> io::Result<bool> {
        p.recheck_processes()?;
        if herdr::process_evidence(self.herdr.pid)? != self.herdr
            || identity(&self.herdr_socket)? != self.herdr_object
        {
            return Err(failure("native witness Herdr source changed"));
        }
        let current = Manifest::load(&self.manifest_path)
            .map_err(|_| failure("native witness manifest unavailable"))?;
        if serde_json::to_vec(&current)? != self.manifest_bytes {
            return Err(failure("native witness manifest changed"));
        }
        let observed = current
            .presence_observation(&self.manifest_path)
            .map_err(|_| failure("native witness presence unavailable"))?;
        let (presence, fresh) = match observed {
            goatr::codex_relay::ValidatedPresence::Fresh(presence) => (presence, true),
            goatr::codex_relay::ValidatedPresence::Past(presence)
            | goatr::codex_relay::ValidatedPresence::Unavailable(presence) => (presence, false),
        };
        if presence.state != goatr::codex_relay::PresenceState::Connected
            || serde_json::to_value(&presence.connection_epoch)? != p.objects["connectionEpoch"]
            || presence.tui.as_ref() != p.tui.as_ref()
            || Some(&presence.relay) != p.relay.as_ref()
            || !presence.associated_thread_ids.contains(&p.native)
        {
            return Err(failure("native witness original TUI association changed"));
        }
        if self
            .epoch
            .validate(&self.owner.socket_host, &self.owner.socket_agent)
            .map_err(|_| failure("native witness owner epoch unavailable"))?
            != p.owner
        {
            return Err(failure("native witness owner epoch changed"));
        }
        for process in std::iter::once(&p.owner)
            .chain(p.tui.iter())
            .chain(p.relay.iter())
        {
            let status = fs::read_to_string(format!("/proc/{}/status", process.pid))?;
            for key in ["CapEff:", "CapAmb:"] {
                let value = status
                    .lines()
                    .find_map(|line| line.strip_prefix(key))
                    .and_then(|value| u64::from_str_radix(value.trim(), 16).ok());
                if value != Some(0) {
                    return Err(failure("native witness child capability guard failed"));
                }
            }
        }
        Ok(fresh)
    }
    async fn physical(&self, p: &NativeProof, work: &ReadWork) -> io::Result<()> {
        loop {
            work.check()?;
            // Pending freshness is comparison metadata only. Every observation
            // repeats all original binding/process/socket/owner/capability fences.
            let fresh = self.physical_observation(p)?;
            work.check()?;
            if fresh {
                return Ok(());
            }
            // Only typed Past/Unavailable reach this wait; identity errors and
            // disconnects fail immediately. Never restart the original budget.
            tokio::time::sleep_until(
                work.deadline
                    .min(tokio::time::Instant::now() + Duration::from_millis(100)),
            )
            .await;
        }
    }
    pub(super) async fn connect(p: &NativeProof, inputs: &NativeInputs<'_>) -> io::Result<Self> {
        if p.provider != Provider::Codex {
            return Err(failure("native witness requires original Codex"));
        }
        let path = p
            .manifest
            .as_ref()
            .ok_or_else(|| failure("native witness manifest missing"))?
            .clone();
        let manifest =
            Manifest::load(&path).map_err(|_| failure("native witness manifest unavailable"))?;
        let owner = inputs
            .config
            .codex_owners
            .iter()
            .find(|owner| owner.id == manifest.owner_id)
            .ok_or_else(|| failure("native witness configured owner missing"))?
            .clone();
        if manifest.expected_thread_id != p.native
            || serde_json::to_value(&manifest.owner_epoch)? != p.objects["ownerEpoch"]
            || serde_json::to_value(&manifest.owner_id)? != p.objects["ownerId"]
            || serde_json::to_value(&manifest.binding_id)? != p.objects["bindingId"]
        {
            return Err(failure("native witness original binding mismatch"));
        }
        let epoch = manifest.owner_epoch.clone();
        let (socket, process) = goatr::codex_relay::owner_stream(
            &owner.socket_host,
            &owner.socket_agent,
            &epoch,
            goatr::codex_relay::websocket_config(8 * 1024 * 1024, RPC_BYTES),
        )
        .await
        .map_err(|_| failure("native witness original owner connection failed"))?;
        if process != p.owner {
            return Err(failure("native witness socket process mismatch"));
        }
        let mut witness = Self {
            socket,
            manifest_bytes: serde_json::to_vec(&manifest)?,
            manifest_path: path,
            owner,
            epoch,
            herdr: inputs.herdr.clone(),
            herdr_socket: inputs.socket.to_owned(),
            herdr_object: identity(inputs.socket)?,
            reads: 0,
            calls: 0,
            bytes: 0,
        };
        let mut work = ReadWork::new(tokio::time::Instant::now() + Duration::from_secs(14));
        witness.physical(p, &work).await?;
        let ready=witness.rpc("initialize",json!({"clientInfo":{"name":"goatr_paging_witness","title":"Goatr independent paging witness","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true,"requestAttestation":false,"optOutNotificationMethods":[]}}),&mut work).await?;
        if !ready.is_object() {
            return Err(failure("native witness initialize result invalid"));
        }
        tokio::time::timeout_at(
            work.deadline,
            witness.socket.send(Message::Text(
                json!({"method":"initialized"}).to_string().into(),
            )),
        )
        .await
        .map_err(|_| failure("native witness initialized deadline"))?
        .map_err(|_| failure("native witness initialized transport"))?;
        witness.physical(p, &work).await?;
        Ok(witness)
    }
    async fn rpc(&mut self, method: &str, params: Value, work: &mut ReadWork) -> io::Result<Value> {
        // No resume, shell, turn/start, fork, settings or approval methods exist here.
        if !matches!(
            method,
            "initialize" | "thread/loaded/list" | "thread/read" | "thread/items/list"
        ) {
            return Err(failure("native witness method not permitted"));
        }
        work.check()?;
        if work.calls >= 64 {
            return Err(failure("native witness RPC bound exhausted"));
        }
        work.calls += 1;
        self.calls += 1;
        let id = Uuid::random().0;
        let bytes = json!({"id":id,"method":method,"params":params}).to_string();
        tokio::time::timeout_at(work.deadline, self.socket.send(Message::Text(bytes.into())))
            .await
            .map_err(|_| failure("native witness send deadline"))?
            .map_err(|_| failure("native witness send transport"))?;
        for _ in 0..256 {
            let frame = tokio::time::timeout_at(work.deadline, self.socket.next())
                .await
                .map_err(|_| failure("native witness reply deadline"))?
                .ok_or_else(|| failure("native witness original connection closed"))?
                .map_err(|_| failure("native witness reply transport"))?;
            let payload = match &frame {
                Message::Text(text) => text.as_bytes(),
                Message::Binary(bytes) => bytes.as_ref(),
                Message::Ping(_) | Message::Pong(_) => continue,
                _ => return Err(failure("native witness connection lost")),
            };
            work.bytes = work
                .bytes
                .checked_add(payload.len())
                .ok_or_else(|| failure("native witness byte overflow"))?;
            self.bytes = self.bytes.saturating_add(payload.len());
            work.check()?;
            let value = goatr::native_json::parse(payload, goatr::native_json::NATIVE)
                .map_err(|_| failure("native witness native frame invalid"))?;
            if value.get("method").is_some() {
                if value.get("id").is_some() {
                    return Err(failure("read-only native witness received server request"));
                }
                continue;
            }
            if value["id"].as_str() != Some(&id)
                || value.get("result").is_none()
                || value.get("error").is_some()
            {
                return Err(failure("native witness response rejected or uncorrelated"));
            }
            return Ok(value["result"].clone());
        }
        Err(failure("native witness response frame bound exceeded"))
    }
    async fn loaded(&mut self, p: &NativeProof, work: &mut ReadWork) -> io::Result<()> {
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let mut ids = HashSet::new();
        let mut found = false;
        loop {
            let mut params = json!({"limit":100});
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            let page = self.rpc("thread/loaded/list", params, work).await?;
            let data = page["data"]
                .as_array()
                .ok_or_else(|| failure("native witness loaded list invalid"))?;
            if data.len() > 100 {
                return Err(failure("native witness loaded list bound"));
            }
            for id in data {
                let id = literal_id(id)?;
                if !ids.insert(id.clone()) {
                    return Err(failure("native witness duplicate loaded ID"));
                }
                found |= id == p.native;
            }
            cursor = cursor_value(&page)?;
            match &cursor {
                None => break,
                Some(cursor) if seen.insert(cursor.clone()) => {}
                _ => return Err(failure("native witness loaded cursor cycle")),
            }
        }
        if !found {
            return Err(failure("native witness original thread no longer loaded"));
        }
        Ok(())
    }
    async fn read(
        &mut self,
        p: &NativeProof,
        method: &str,
        params: Value,
        deadline: tokio::time::Instant,
    ) -> io::Result<Value> {
        let mut work =
            ReadWork::new(deadline.min(tokio::time::Instant::now() + Duration::from_secs(14)));
        self.physical(p, &work).await?;
        work.check()?;
        self.loaded(p, &mut work).await?;
        let result = self.rpc(method, params, &mut work).await?;
        self.loaded(p, &mut work).await?;
        self.physical(p, &work).await?;
        work.check()?;
        self.reads += 1;
        Ok(result)
    }
    async fn head(&mut self, p: &NativeProof, deadline: tokio::time::Instant) -> io::Result<Value> {
        self.read(
            p,
            "thread/items/list",
            json!({"threadId":p.native.0,"limit":100,"sortDirection":"desc"}),
            deadline,
        )
        .await
    }
    pub(super) async fn admit(&mut self, p: &NativeProof, zero: bool) -> io::Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let metadata = self
            .read(
                p,
                "thread/read",
                json!({"threadId":p.native.0,"includeTurns":false}),
                deadline,
            )
            .await?;
        if literal_id(&metadata["thread"]["id"])? != p.native
            || metadata["thread"]["historyMode"] != "paginated"
        {
            return Err(failure("native witness requires proven paginated thread"));
        }
        let page = self.head(p, deadline).await?;
        validate_entries(&page)?;
        if zero
            && (!page["data"].as_array().expect("validated page").is_empty()
                || cursor_value(&page)?.is_some())
        {
            return Err(failure(
                "independent paging proof requires new zero-item thread",
            ));
        }
        Ok(())
    }
    pub(super) async fn completed(
        &mut self,
        p: &NativeProof,
        command: &str,
        marker: &str,
        deadline: tokio::time::Instant,
    ) -> io::Result<(Id, Id)> {
        for observation in 0..16 {
            if observation > 0 {
                tokio::time::timeout_at(deadline, tokio::time::sleep(Duration::from_secs(2)))
                    .await
                    .map_err(|_| failure("native witness completion deadline"))?;
            }
            let page = self.head(p, deadline).await?;
            if let Some(id) = command_completion(&page, command, marker)? {
                return Ok(id);
            }
        }
        Err(failure("native witness completion observation bound"))
    }
}
fn literal_id(value: &Value) -> io::Result<Id> {
    let id = Id(value
        .as_str()
        .ok_or_else(|| failure("native witness literal ID missing"))?
        .to_owned());
    id.validate()
        .map_err(|_| failure("native witness literal ID invalid"))?;
    Ok(id)
}
fn cursor_value(page: &Value) -> io::Result<Option<String>> {
    match page.get("nextCursor") {
        Some(Value::Null) => Ok(None),
        Some(Value::String(cursor)) if !cursor.is_empty() && cursor.len() <= 65_536 => {
            Ok(Some(cursor.clone()))
        }
        _ => Err(failure("native witness cursor invalid")),
    }
}
fn validate_entries(page: &Value) -> io::Result<()> {
    cursor_value(page)?;
    let data = page["data"]
        .as_array()
        .ok_or_else(|| failure("native witness item page missing"))?;
    if data.len() > 100 {
        return Err(failure("native witness item count bound"));
    }
    let mut ids = HashSet::new();
    for entry in data {
        literal_id(&entry["turnId"])?;
        let id = literal_id(&entry["item"]["id"])?;
        if !ids.insert(id) {
            return Err(failure("native witness duplicate item"));
        }
        match entry.get("completedAtMs") {
            Some(Value::Null) => {}
            Some(value) if value.as_u64().is_some_and(|n| n <= UINT_MAX) => {}
            _ => return Err(failure("native witness lifecycle invalid")),
        }
    }
    Ok(())
}
fn command_completion(page: &Value, command: &str, marker: &str) -> io::Result<Option<(Id, Id)>> {
    validate_entries(page)?;
    let mut found = None;
    for entry in page["data"].as_array().expect("validated page") {
        let item = &entry["item"];
        if item["type"] != "commandExecution" {
            continue;
        }
        let Some(output) = item["aggregatedOutput"].as_str() else {
            continue;
        };
        if !output.lines().any(|line| line == marker) {
            continue;
        }
        if !item["command"]
            .as_str()
            .is_some_and(|display| codex_shell_display_matches(display, command))
            || output.lines().filter(|line| *line == marker).count() != 1
        {
            return Err(failure("native witness command or marker mismatch"));
        }
        if entry["completedAtMs"].is_null() || item["status"] == "inProgress" {
            continue;
        }
        if item["status"] != "completed"
            || item["exitCode"].as_i64() != Some(0)
            || item.get("truncated").is_some_and(|v| v != false)
            || item.get("outputTruncated").is_some_and(|v| v != false)
        {
            return Err(failure(
                "native witness command did not complete successfully",
            ));
        }
        if found
            .replace((literal_id(&item["id"])?, literal_id(&entry["turnId"])?))
            .is_some()
        {
            return Err(failure("native witness duplicate command completion"));
        }
    }
    Ok(found)
}
fn stable_browse_page(left: &SessionHistoryResult, right: &SessionHistoryResult) -> bool {
    left.view_id == right.view_id
        && left.next_cursor == right.next_cursor
        && left.records == right.records
}

pub(super) async fn run(
    phone: &mut Phone,
    p: &NativeProof,
    inputs: &NativeInputs<'_>,
    mode: NativeHistoryMode,
) -> io::Result<Value> {
    if !inputs.enabled || p.provider != Provider::Codex {
        return Err(failure(
            "independent Codex paging requires explicit guarded input consent",
        ));
    }
    let commands_required = mode.command_count();
    let zero = mode.diagnostic();
    inputs.current(phone, p).await?;
    let mut witness = Witness::connect(p, inputs).await?;
    witness.admit(p, zero).await?;
    let initial = phone.start_generation(&p.target).await?;
    if zero && !initial.history.is_empty() {
        return Err(failure("new paging thread has prior Goatr history"));
    }
    let started = Instant::now();
    let phase = started + Duration::from_secs(3600);
    let mut confirmed: HashMap<Id, (Id, usize)> = HashMap::new();
    let mut ordered = Vec::with_capacity(commands_required);
    let mut latest = initial;
    for index in 0..commands_required {
        if Instant::now() >= phase {
            return Err(failure("independent paging generation capacity exceeded"));
        }
        if latest.state.summary.connectivity != SessionConnectivity::Ready
            || latest.state.summary.status != SessionStatus::Idle
            || !latest.pending_requests.is_empty()
        {
            return Err(generation_idle_failure(index, false, false, false));
        }
        let marker = format!("goatr-history-{}-Codex-{index}", &inputs.run.0[..8]);
        let command = format!("printf '%s\\n' '{marker}'");
        let command_started = Instant::now();
        let outer =
            history_command_deadline(inputs.paging_proof, Provider::Codex, phase, command_started);
        phone.history_command = Some(HistoryCommandTrace::default());
        let completion = tokio::time::timeout_at(outer.into(), async {
            inputs
                .send(
                    phone,
                    p,
                    NativeInput::Prompt(&format!("!{command}")),
                    None,
                    "native_history_command",
                )
                .await?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            let ((snapshot, id), (native, turn)) = tokio::try_join!(
                async {
                    tokio::time::timeout_at(
                        deadline,
                        inputs.shell_completed(phone, p, &command, &marker),
                    )
                    .await
                    .map_err(|_| failure("Goatr completion deadline; no replay"))?
                },
                witness.completed(p, &command, &marker, deadline)
            )?;
            if id != native
                || snapshot
                    .history
                    .iter()
                    .find(|record| record.id == id)
                    .and_then(|record| record.turn_id.as_ref())
                    != Some(&turn)
            {
                return Err(failure(
                    "independent native and adapter completion IDs differ",
                ));
            }
            Ok::<_, io::Error>((snapshot, id, turn))
        })
        .await;
        let (snapshot, id, turn) = match completion {
            Ok(result) => result?,
            Err(_) => {
                return Err(phone
                    .history_command
                    .as_ref()
                    .expect("command trace")
                    .deadline());
            }
        };
        phone.history_command = None;
        if confirmed
            .insert(id.clone(), (turn.clone(), index))
            .is_some()
        {
            return Err(failure("independent native command reused an ID"));
        }
        ordered.push((id, turn));
        latest = snapshot;
        let commands = index + 1;
        if commands == 16 || commands == 32 || commands.is_multiple_of(64) {
            write_json(
                &inputs.evidence.join(format!(
                    "native-history-progress-{}-Codex-{commands}.json",
                    inputs.run.0
                )),
                &json!({"runId":inputs.run,"target":p.target,"commands":commands,"distinctCompletedIds":confirmed.len(),"independentNativeCompletedIds":confirmed.len(),"elapsedMs":started.elapsed().as_millis(),"witnessReads":witness.reads,"witnessRpcs":witness.calls,"witnessBytes":witness.bytes,"payloads":"not retained"}),
            )?;
        }
        if commands == 16
            && started
                .elapsed()
                .as_millis()
                .saturating_mul(commands_required as u128)
                > 3600 * 1000 * 16
        {
            return Err(failure(
                "measured paging generation cannot fit its unchanged phase budget",
            ));
        }
    }
    // An ordinary framed barrier drains the original subscription; no history read.
    phone.host().await?;
    let view = phone
        .generation
        .as_ref()
        .ok_or_else(|| failure("original generation view missing"))?;
    if view.failed
        || target(&view.snapshot.state.summary) != p.target
        || view.snapshot.state.summary.connectivity != SessionConnectivity::Ready
        || view.snapshot.state.summary.status != SessionStatus::Idle
        || !view.snapshot.pending_requests.is_empty()
    {
        return Err(failure(
            "generation checkpoint not original completed idle boundary",
        ));
    }
    let retained: HashSet<_> = view
        .snapshot
        .history
        .iter()
        .filter(|record| record.complete)
        .map(|record| record.id.clone())
        .collect();
    if retained.len() > 1000 || confirmed.len() != commands_required {
        return Err(failure("generation retained/witness count invalid"));
    }
    let ledger: Vec<_> = ordered
        .iter()
        .enumerate()
        .map(|(index, (id, turn))| json!({"id":id,"turnId":turn,"commandIndex":index}))
        .collect();
    write_json(
        &inputs.evidence.join(format!(
            "native-history-generation-{}-Codex.json",
            inputs.run.0
        )),
        &json!({"runId":inputs.run,"target":p.target,"proofMode":mode.name(),"commands":confirmed.len(),"distinctCompletedIds":confirmed.len(),"independentNativeCompletedIds":confirmed.len(),"generationComplete":true,"olderPagingProven":false,"retainedRecordLimit":1000,"retainedIdUpperBound":retained,"nativeCompletionLedger":ledger,"ready":true,"idle":true,"pendingEmpty":true,"elapsedMs":started.elapsed().as_millis(),"witnessReads":witness.reads,"witnessRpcs":witness.calls,"witnessBytes":witness.bytes,"nativeSourceEpoch":p.objects,"payloads":"not retained"}),
    )?;
    let visible: HashSet<_> = view
        .snapshot
        .history
        .iter()
        .rev()
        .take(100)
        .map(|record| record.id.clone())
        .collect();
    let paging_deadline = Instant::now() + Duration::from_secs(15 * 60);
    let mut cursor: Option<Id> = None;
    let mut view_id: Option<Uuid> = None;
    let mut seen = HashSet::new();
    let mut matched = HashSet::new();
    let mut returned = HashSet::new();
    let mut outside = Vec::new();
    let mut repeats = 0;
    let mut pages = 0;
    let mut previous_oldest = usize::MAX;
    let mut append_cursor_page: Option<(SessionHistoryParams, SessionHistoryResult)> = None;
    let mut qualified: Option<(Id, usize)> = None;
    loop {
        if pages >= 64 || Instant::now() >= paging_deadline {
            return Err(failure("independent native browse work bound exceeded"));
        }
        if let Some(cursor) = &cursor
            && !seen.insert(cursor.clone())
        {
            return Err(failure("independent native browse cursor cycle"));
        }
        let params = SessionHistoryParams {
            target: p.target.clone(),
            view_id: view_id.clone(),
            cursor: cursor.clone(),
            limit: Some(Uint(100)),
        };
        let ResultBody::SessionHistory(page) = phone
            .rpc(RequestBody::SessionHistory(params.clone()), "history_page")
            .await?
        else {
            return Err(failure("native browse response missing"));
        };
        if view_id.as_ref().is_some_and(|view| view != &page.view_id) {
            return Err(failure("native browse view changed"));
        }
        view_id = Some(page.view_id.clone());
        let mut indices = Vec::new();
        for record in &page.records {
            if !returned.insert(record.id.clone()) {
                return Err(failure("native browse duplicated a row"));
            }
            if !record.complete || record.source != RecordSource::NativePage {
                return Err(failure(
                    "native browse returned non-completed/non-native row",
                ));
            }
            if let Some((turn, index)) = confirmed.get(&record.id) {
                if record.turn_id.as_ref() != Some(turn) || !matched.insert(record.id.clone()) {
                    return Err(failure(
                        "native browse independent ID/turn correlation failed",
                    ));
                }
                indices.push(*index);
                if !visible.contains(&record.id) && outside.len() < 16 {
                    outside.push(record.id.clone());
                }
                let later = commands_required
                    .checked_sub(*index + 1)
                    .ok_or_else(|| failure("native completion order overflow"))?;
                if !mode.diagnostic()
                    && !retained.contains(&record.id)
                    && later >= 1000
                    && qualified.is_none()
                {
                    qualified = Some((record.id.clone(), later));
                    if !outside.contains(&record.id) {
                        if outside.len() == 16 {
                            outside.pop();
                        }
                        outside.push(record.id.clone());
                    }
                }
            }
        }
        if indices.windows(2).any(|w| w[0] >= w[1])
            || indices
                .last()
                .is_some_and(|newest| *newest >= previous_oldest)
        {
            return Err(failure(
                "native browse disagrees with witnessed creation order",
            ));
        }
        if let Some(oldest) = indices.first() {
            previous_oldest = *oldest;
        }
        if cursor.is_some() {
            let ResultBody::SessionHistory(repeated) = phone
                .rpc(
                    RequestBody::SessionHistory(params.clone()),
                    "history_page_repeat",
                )
                .await?
            else {
                return Err(failure("native browse repeat missing"));
            };
            if !stable_browse_page(&page, &repeated) {
                return Err(failure(
                    "native browse repeat changed membership order display or successor",
                ));
            }
            repeats += 1;
        }
        if append_cursor_page.is_none()
            && params.cursor.is_some()
            && params.view_id.as_ref() == Some(&page.view_id)
            && !page.records.is_empty()
            && (if mode.diagnostic() {
                page.records.iter().any(|record| {
                    confirmed.contains_key(&record.id) && !visible.contains(&record.id)
                })
            } else {
                qualified
                    .as_ref()
                    .is_some_and(|(id, _)| page.records.iter().any(|record| &record.id == id))
            })
        {
            append_cursor_page = Some((params, page.clone()));
        }
        pages += 1;
        cursor = page.next_cursor;
        if cursor.is_none() {
            if matched.len() != commands_required
                || repeats == 0
                || outside.len() < 16
                || (!mode.diagnostic() && qualified.is_none())
            {
                return Err(failure(
                    "bounded native paging did not return the independently confirmed commands and a stable continuation",
                ));
            }
            break;
        }
    }
    let (append_params, append_page) = append_cursor_page
        .ok_or_else(|| failure("bounded native paging did not capture an explicit cursor page"))?;
    if Instant::now() >= phase
        || Instant::now() >= paging_deadline
        || latest.state.summary.connectivity != SessionConnectivity::Ready
        || latest.state.summary.status != SessionStatus::Idle
        || !latest.pending_requests.is_empty()
    {
        return Err(failure(
            "live append requires the original completed idle generation boundary",
        ));
    }
    let append_marker = format!("goatr-history-{}-Codex-live-append", &inputs.run.0[..8]);
    let append_command = format!("printf '%s\\n' '{append_marker}'");
    let append_started = Instant::now();
    let append_outer =
        history_command_deadline(inputs.paging_proof, Provider::Codex, phase, append_started)
            .min(paging_deadline);
    phone.history_command = Some(HistoryCommandTrace::default());
    let append_completion = tokio::time::timeout_at(append_outer.into(), async {
        inputs
            .send(
                phone,
                p,
                NativeInput::Prompt(&format!("!{append_command}")),
                None,
                "native_history_live_append",
            )
            .await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let ((snapshot, id), (native, turn)) = tokio::try_join!(
            async {
                tokio::time::timeout_at(
                    deadline,
                    inputs.shell_completed(phone, p, &append_command, &append_marker),
                )
                .await
                .map_err(|_| failure("Goatr live append completion deadline; no replay"))?
            },
            witness.completed(p, &append_command, &append_marker, deadline)
        )?;
        if id != native
            || snapshot
                .history
                .iter()
                .find(|record| record.id == id)
                .and_then(|record| record.turn_id.as_ref())
                != Some(&turn)
        {
            return Err(failure(
                "independent native and adapter live append IDs differ",
            ));
        }
        Ok::<_, io::Error>((snapshot, id, turn))
    })
    .await;
    let (append_snapshot, append_id, append_turn) = match append_completion {
        Ok(result) => result?,
        Err(_) => {
            return Err(phone
                .history_command
                .as_ref()
                .expect("command trace")
                .deadline());
        }
    };
    phone.history_command = None;
    if confirmed.contains_key(&append_id)
        || target(&append_snapshot.state.summary) != p.target
        || append_snapshot.state.summary.connectivity != SessionConnectivity::Ready
        || append_snapshot.state.summary.status != SessionStatus::Idle
        || !append_snapshot.pending_requests.is_empty()
        || append_snapshot
            .history
            .iter()
            .find(|record| record.id == append_id)
            .and_then(|record| record.turn_id.as_ref())
            != Some(&append_turn)
    {
        return Err(failure(
            "live append did not complete on the original target/identity boundary",
        ));
    }
    if append_params.target != p.target
        || append_params.cursor.is_none()
        || append_params.view_id.as_ref() != Some(&append_page.view_id)
    {
        return Err(failure(
            "captured native browse cursor did not retain original view identity",
        ));
    }
    let after_append = tokio::time::timeout_at(
        paging_deadline.into(),
        phone.rpc(
            RequestBody::SessionHistory(append_params),
            "history_page_live_append_repeat",
        ),
    )
    .await
    .map_err(|_| failure("native browse live append repeat exceeded original view expiry"))??;
    let ResultBody::SessionHistory(after_append) = after_append else {
        return Err(failure("native browse live append repeat missing"));
    };
    if !stable_browse_page(&append_page, &after_append) {
        return Err(failure(
            "native browse live append changed captured membership order display or successor",
        ));
    }
    inputs.current(phone, p).await?;
    let final_work = ReadWork::new(
        tokio::time::Instant::from_std(paging_deadline)
            .min(tokio::time::Instant::now() + Duration::from_secs(14)),
    );
    witness.physical(p, &final_work).await?;
    if !zero {
        phone.finish_generation(&p.target).await?;
    }
    let (qualified_id, qualified_later_completed_lower_bound) = match qualified {
        Some((id, later)) => (Some(id), Some(later)),
        None if mode.diagnostic() => (None, None),
        None => return Err(failure("strict native boundary witness missing")),
    };
    Ok(
        json!({"scenario":"independent-native-paging-live-append","target":p.target,"provider":p.provider,"proofMode":mode.name(),"viewId":view_id,"totalPages":pages,"maxCursorAdvances":64,"viewExpirySeconds":900,"stableCursorRepeats":repeats,"liveAppendCursorStable":true,"generationCommands":confirmed.len(),"liveAppendCommands":1,"totalNativeCommands":confirmed.len()+1,"distinctCompletedRecords":matched.len(),"totalNativeRecords":returned.len(),"independentNativeCompletedIds":confirmed.len()+1,"localWindowLimit":goatr::state::HISTORY_RECORDS,"retainedIdUpperBound":retained,"nativeOlderReturnedIds":outside,"qualifiedNativeId":qualified_id,"qualifiedLaterCompletedLowerBound":qualified_later_completed_lower_bound,"nativeOlderReadLowerBound":1,"nativeOlderSource":"codex_thread_wide_items_list","nativeSourceEpoch":p.objects,"generationElapsedMs":started.elapsed().as_millis(),"generationReadPolicy":"independent read-only original owner head pages and one ordered Goatr subscription; no resume or mutation","witnessReads":witness.reads,"witnessRpcs":witness.calls,"witnessBytes":witness.bytes,"boundedPagingProven":true,"strictRetentionBoundary":!mode.diagnostic(),"exhaustiveRetentionCrossing":if mode.diagnostic(){"out_of_scope"}else{"proved"},"noContentOriginalsInEvidence":true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(completed: bool) -> Value {
        json!({"data":[{"turnId":"turn-1","completedAtMs":if completed{json!(1)}else{Value::Null},"item":{"id":"literal-native-id","type":"commandExecution","command":"/bin/bash -lc 'printf marker'","aggregatedOutput":"marker\n","status":if completed{"completed"}else{"inProgress"},"exitCode":0}}],"nextCursor":null})
    }
    #[test]
    fn native_witness_requires_literal_completed_command_not_ack_or_projection() {
        assert!(
            command_completion(&page(false), "printf marker", "marker")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            command_completion(&page(true), "printf marker", "marker").unwrap(),
            Some((Id("literal-native-id".into()), Id("turn-1".into())))
        );
        for fault in [
            "exit",
            "command",
            "marker",
            "truncated",
            "duplicate",
            "lifecycle",
        ] {
            let mut value = page(true);
            match fault {
                "exit" => value["data"][0]["item"]["exitCode"] = json!(1),
                "command" => {
                    value["data"][0]["item"]["command"] = json!("/bin/bash -lc 'different'")
                }
                "marker" => {
                    value["data"][0]["item"]["aggregatedOutput"] = json!("marker\nmarker\n")
                }
                "truncated" => value["data"][0]["item"]["truncated"] = json!(true),
                "duplicate" => {
                    let entry = value["data"][0].clone();
                    value["data"].as_array_mut().unwrap().push(entry);
                }
                _ => value["data"][0]["completedAtMs"] = json!("private-invalid"),
            };
            assert!(
                command_completion(&value, "printf marker", "marker").is_err(),
                "{fault}"
            );
        }
    }
}
