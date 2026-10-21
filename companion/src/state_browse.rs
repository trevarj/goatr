// SPDX-License-Identifier: GPL-3.0-or-later
//! Native browse views share the live history owner's accounting, never its order.
use super::*;
use std::any::Any;

#[derive(Clone)]
pub struct BrowsePosition {
    value: Arc<dyn Any + Send + Sync>,
    bytes: usize,
}
impl BrowsePosition {
    pub fn new<T: Any + Send + Sync>(value: T, bytes: usize) -> StateResult<Self> {
        if bytes > SESSION_BYTES {
            return Err(ErrorCode::ResourceLimit);
        }
        Ok(Self {
            value: Arc::new(value),
            bytes,
        })
    }
    pub fn get<T: Any>(&self) -> StateResult<&T> {
        self.value.downcast_ref().ok_or(ErrorCode::CursorExpired)
    }
    fn identity(&self) -> usize {
        Arc::as_ptr(&self.value) as *const () as usize
    }
}
struct CapturedPage {
    records: HistorySnapshot,
    next: Option<BrowsePosition>,
}
pub(super) struct BrowseNode {
    pub(super) peer: Uuid,
    pub(super) target: Target,
    pub(super) source: Uuid,
    pub(super) view: Uuid,
    pub(super) expires: u64,
    position: Option<BrowsePosition>,
    captured: Option<Arc<CapturedPage>>,
    end: usize,
    pages: HashMap<usize, SessionHistoryResult>,
    gate: Arc<tokio::sync::Mutex<()>>,
}
#[derive(Clone)]
pub struct BrowseTicket {
    token: Uuid,
    pub(super) peer: Uuid,
    pub(super) target: Target,
    pub(super) source: Uuid,
    pub(super) view: Uuid,
    limit: usize,
    position: BrowsePosition,
    gate: Arc<tokio::sync::Mutex<()>>,
}
impl BrowseTicket {
    pub fn position<T: Any>(&self) -> StateResult<&T> {
        self.position.get()
    }
    pub fn gate(&self) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(&self.gate)
    }
    pub fn view_id(&self) -> &Uuid {
        &self.view
    }
    pub fn source(&self) -> &Uuid {
        &self.source
    }
    pub fn target(&self) -> &Target {
        &self.target
    }
    pub fn limit(&self) -> usize {
        self.limit
    }
    pub fn cursor(&self) -> Id {
        Id(self.token.0.clone())
    }
}
pub enum BrowseLookup {
    Cached(SessionHistoryResult),
    Fetch(BrowseTicket),
}

pub(super) fn storage_bytes(i: &Inner) -> usize {
    let mut pages = HashSet::new();
    let mut positions = HashSet::new();
    i.browse
        .values()
        .map(|node| {
            let mut bytes = 256
                + node
                    .pages
                    .values()
                    .map(|p| p.records.iter().map(record_bytes).sum::<usize>() + 128)
                    .sum::<usize>();
            if let Some(position) = &node.position
                && positions.insert(position.identity())
            {
                bytes += position.bytes;
            }
            if let Some(page) = &node.captured
                && pages.insert(Arc::as_ptr(page) as usize)
            {
                bytes += page.records.bytes;
                if let Some(position) = &page.next
                    && positions.insert(position.identity())
                {
                    bytes += position.bytes;
                }
            }
            bytes
        })
        .sum()
}
fn reserve(i: &mut Inner, keep: &Uuid, bytes: usize, nodes: usize, now: u64) -> StateResult<()> {
    i.browse.retain(|_, n| now < n.expires);
    loop {
        if i.browse.len() + nodes <= 64
            && history_storage_bytes(i).saturating_add(bytes) <= SESSION_BYTES
        {
            return Ok(());
        }
        let victim = i
            .browse
            .values()
            .filter(|n| &n.view != keep)
            .min_by_key(|n| n.expires)
            .map(|n| n.view.clone());
        let Some(victim) = victim else {
            return Err(ErrorCode::ResourceLimit);
        };
        i.browse.retain(|_, n| n.view != victim);
    }
}
fn check(
    i: &Inner,
    token: &Uuid,
    peer: &Uuid,
    target: &Target,
    view: &Uuid,
    now: u64,
) -> StateResult<()> {
    let source = &current(i, target)
        .map_err(|_| ErrorCode::CursorExpired)?
        .source;
    let n = i.browse.get(token).ok_or(ErrorCode::CursorExpired)?;
    if &n.peer != peer
        || &n.target != target
        || &n.source != source
        || &n.view != view
        || now >= n.expires
    {
        return Err(ErrorCode::CursorExpired);
    }
    Ok(())
}
fn page(i: &mut Inner, token: &Uuid, limit: usize, now: u64) -> StateResult<SessionHistoryResult> {
    let node = i.browse.get(token).ok_or(ErrorCode::CursorExpired)?;
    if let Some(page) = node.pages.get(&limit) {
        return Ok(page.clone());
    }
    let captured = Arc::clone(node.captured.as_ref().ok_or(ErrorCode::NotReady)?);
    let (rows, end) = latest_page(&captured.records, node.end, limit)?;
    let (peer, target, source, view, expires) = (
        node.peer.clone(),
        node.target.clone(),
        node.source.clone(),
        node.view.clone(),
        node.expires,
    );
    let next_position = if end == 0 {
        captured.next.clone()
    } else {
        None
    };
    let need_next = end > 0 || next_position.is_some();
    let existing = if need_next {
        i.browse.iter().find_map(|(id, n)| {
            (n.view == view
                && (if end > 0 {
                    n.end == end
                        && n.captured
                            .as_ref()
                            .is_some_and(|p| Arc::ptr_eq(p, &captured))
                } else {
                    n.position
                        .as_ref()
                        .zip(next_position.as_ref())
                        .is_some_and(|(a, b)| a.identity() == b.identity())
                }))
            .then(|| id.clone())
        })
    } else {
        None
    };
    let next = existing.clone().or_else(|| need_next.then(Uuid::random));
    let result = SessionHistoryResult {
        view_id: view.clone(),
        records: rows,
        next_cursor: next.as_ref().map(|n| Id(n.0.clone())),
    };
    // Reserve successor and memoized response together, before publishing either.
    reserve(
        i,
        &view,
        result.records.iter().map(record_bytes).sum::<usize>() + 384,
        usize::from(need_next && existing.is_none()),
        now,
    )?;
    frame(&Envelope::Response(Response {
        id: Uuid::random(),
        outcome: Ok(ResultBody::SessionHistory(result.clone())),
    }))?;
    if need_next && existing.is_none() {
        i.browse.insert(
            next.expect("reserved successor"),
            BrowseNode {
                peer,
                target,
                source,
                view,
                expires,
                position: next_position,
                captured: (end > 0).then(|| Arc::clone(&captured)),
                end,
                pages: HashMap::new(),
                gate: Arc::new(tokio::sync::Mutex::new(())),
            },
        );
    }
    i.browse
        .get_mut(token)
        .ok_or(ErrorCode::CursorExpired)?
        .pages
        .insert(limit, result.clone());
    Ok(result)
}
impl State {
    pub fn begin_browse(
        &self,
        peer: &AuthenticatedPeer,
        target: &Target,
        source: &Uuid,
        position: BrowsePosition,
        limit: usize,
        now: u64,
    ) -> StateResult<BrowseTicket> {
        if !(1..=100).contains(&limit) {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut i = self.inner.lock().expect("state mutex");
        let session = current(&i, target)?;
        if &session.source != source || session.installing {
            return Err(ErrorCode::NotReady);
        }
        let view = Uuid::random();
        reserve(&mut i, &view, position.bytes + 256, 1, now)?;
        let token = Uuid::random();
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        i.browse.insert(
            token.clone(),
            BrowseNode {
                peer: peer.peer_id().clone(),
                target: target.clone(),
                source: source.clone(),
                view: view.clone(),
                expires: now.saturating_add(CONTENT_TTL_MS),
                position: Some(position.clone()),
                captured: None,
                end: 0,
                pages: HashMap::new(),
                gate: gate.clone(),
            },
        );
        Ok(BrowseTicket {
            token,
            peer: peer.peer_id().clone(),
            target: target.clone(),
            source: source.clone(),
            view,
            limit,
            position,
            gate,
        })
    }
    pub fn browse_page(
        &self,
        peer: &AuthenticatedPeer,
        target: &Target,
        view: &Uuid,
        cursor: &Id,
        limit: usize,
        now: u64,
    ) -> StateResult<BrowseLookup> {
        if !(1..=100).contains(&limit) {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut i = self.inner.lock().expect("state mutex");
        let token = Uuid(cursor.0.clone());
        check(&i, &token, peer.peer_id(), target, view, now)?;
        if i.browse[&token].captured.is_some() {
            return page(&mut i, &token, limit, now).map(BrowseLookup::Cached);
        }
        let n = &i.browse[&token];
        Ok(BrowseLookup::Fetch(BrowseTicket {
            token,
            peer: n.peer.clone(),
            target: target.clone(),
            source: n.source.clone(),
            view: view.clone(),
            limit,
            position: n.position.clone().ok_or(ErrorCode::CursorExpired)?,
            gate: Arc::clone(&n.gate),
        }))
    }
    pub fn finish_browse(
        &self,
        ticket: BrowseTicket,
        records: Vec<SafeHistory>,
        next: Option<BrowsePosition>,
        now: u64,
    ) -> StateResult<SessionHistoryResult> {
        if records.len() > HISTORY_RECORDS {
            return Err(ErrorCode::ResourceLimit);
        }
        let mut records: Vec<_> = records.into_iter().map(|r| r.0).collect();
        records.sort_by_key(|r| r.order);
        let mut ids = HashSet::new();
        if records
            .iter()
            .any(|r| !r.complete || !ids.insert(r.id.clone()))
            || records.windows(2).any(|w| w[0].order >= w[1].order)
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut i = self.inner.lock().expect("state mutex");
        check(
            &i,
            &ticket.token,
            &ticket.peer,
            &ticket.target,
            &ticket.view,
            now,
        )?;
        if i.browse[&ticket.token].captured.is_none() {
            let captured = Arc::new(CapturedPage {
                records: HistorySnapshot::new(records),
                next,
            });
            let bytes = captured.records.bytes + captured.next.as_ref().map_or(0, |n| n.bytes);
            // One full response plus successor reservation makes installation transactional.
            reserve(
                &mut i,
                &ticket.view,
                bytes.saturating_mul(2) + 1024,
                usize::from(captured.next.is_some()),
                now,
            )?;
            let n = i.browse.get_mut(&ticket.token).expect("validated ticket");
            n.end = captured.records.len();
            n.captured = Some(captured);
        }
        page(&mut i, &ticket.token, ticket.limit, now)
    }
}
