// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded Goatr-owned porcelain/numstat/patch consumers; no native actor or runner.
use super::*;
use std::collections::BTreeSet;
const MAX_ENTRIES: usize = 128;
const MAX_PATH_BYTES: usize = 4096;
const MAX_DISPLAY_BYTES: usize = 1024;
const MAX_PATCH_BYTES: usize = 256 * 1024;
#[derive(Default, PartialEq, Eq)]
pub(super) struct ParsedRepositoryContext {
    pub(super) branch: Option<String>,
    pub(super) head: Option<String>,
    pub(super) detached: bool,
    pub(super) unborn: bool,
    pub(super) upstream: Option<String>,
    pub(super) ahead: Option<u64>,
    pub(super) behind: Option<u64>,
}
pub(super) struct ParsedChanges {
    pub(super) headers: ParsedRepositoryContext,
    pub(super) entries: Vec<ParsedEntry>,
}

pub(super) fn valid_context_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub(super) fn parse_latest_commit(
    bytes: &[u8],
    expected: &str,
    deadline: Instant,
) -> Result<ChangesLatestCommit, ErrorCode> {
    ensure_before(deadline)?;
    let value = std::str::from_utf8(bytes).map_err(|_| ErrorCode::NotReady)?;
    let mut fields = value.split('\0');
    let oid = fields.next().ok_or(ErrorCode::NotReady)?;
    let subject = fields.next().ok_or(ErrorCode::NotReady)?;
    if oid != expected
        || !valid_context_oid(oid)
        || subject.len() > MAX_DISPLAY_BYTES
        || subject.chars().any(char::is_control)
        || fields.next() != Some("")
        || fields.next().is_some()
    {
        return Err(ErrorCode::NotReady);
    }
    Ok(ChangesLatestCommit {
        oid: oid.to_owned(),
        subject: subject.to_owned(),
    })
}

pub(super) enum ParsedEntry {
    Tracked(ParsedTracked),
    Untracked { path: String },
}

pub(super) struct ParsedTracked {
    pub(super) path: String,
    pub(super) original_path: Option<String>,
    pub(super) index: ChangeState,
    pub(super) worktree: ChangeState,
    pub(super) submodule: Option<ChangeSubmodule>,
    pub(super) staged_binary: bool,
    pub(super) unstaged_binary: bool,
    pub(super) staged_numstat_observed: bool,
    pub(super) unstaged_numstat_observed: bool,
    pub(super) staged_conflict_sentinel: bool,
    // Validated porcelain index stages retained for conflict numstat joins.
    pub(super) conflict_stages: Option<u8>,
}

impl ParsedEntry {
    pub(super) fn path(&self) -> &str {
        match self {
            Self::Tracked(entry) => &entry.path,
            Self::Untracked { path } => path,
        }
    }

    pub(super) fn into_entry(self) -> ChangeEntry {
        match self {
            Self::Tracked(entry) => ChangeEntry::Tracked {
                path: entry.path,
                original_path: entry.original_path,
                index: entry.index,
                worktree: entry.worktree,
                submodule: entry.submodule,
                staged_binary: entry.staged_binary,
                unstaged_binary: entry.unstaged_binary,
            },
            Self::Untracked { path } => ChangeEntry::Untracked { path },
        }
    }
}

pub(super) fn parse_status(bytes: &[u8], deadline: Instant) -> Result<ParsedChanges, ErrorCode> {
    ensure_before(deadline)?;
    let output = std::str::from_utf8(bytes).map_err(|_| ErrorCode::NotReady)?;
    let mut headers = ParsedRepositoryContext::default();
    let mut seen_headers = BTreeSet::new();
    let mut entries = Vec::new();

    if bytes.last() != Some(&0) {
        return Err(ErrorCode::NotReady);
    }
    let mut fields = output.split_terminator('\0');
    while let Some(field) = fields.next() {
        ensure_before(deadline)?;
        if field.is_empty() {
            return Err(ErrorCode::NotReady);
        }
        if let Some(header) = field.strip_prefix("# ") {
            parse_header(header, &mut headers, &mut seen_headers)?;
            continue;
        }
        let entry = if let Some(record) = field.strip_prefix("1 ") {
            parse_ordinary(record)?
        } else if let Some(record) = field.strip_prefix("2 ") {
            let original_path = fields.next().ok_or(ErrorCode::NotReady)?;
            parse_rename(record, original_path)?
        } else if let Some(record) = field.strip_prefix("u ") {
            parse_unmerged(record)?
        } else if let Some(path) = field.strip_prefix("? ") {
            validate_path(path)?;
            ParsedEntry::Untracked {
                path: path.to_owned(),
            }
        } else {
            return Err(ErrorCode::NotReady);
        };
        // Historical rename sources are not current paths.  In particular,
        // `old` may be an untracked current path while another entry reports
        // the tracked rename `old -> new`.
        if entries.len() == MAX_ENTRIES {
            return Err(ErrorCode::ResourceLimit);
        }
        entries.push(entry);
    }
    {
        let mut paths = BTreeSet::new();
        for entry in &entries {
            if !paths.insert(entry.path()) {
                return Err(ErrorCode::NotReady);
            }
        }
    }
    if !seen_headers.contains("branch.head")
        || !seen_headers.contains("branch.oid")
        || headers.detached && (headers.branch.is_some() || headers.upstream.is_some())
        || headers.unborn
            && (headers.detached || headers.branch.is_none() || headers.head.is_some())
        || !headers.unborn && headers.head.is_none()
        || seen_headers.contains("branch.ab")
            && (headers.upstream.is_none() || headers.head.is_none())
    {
        return Err(ErrorCode::NotReady);
    }
    Ok(ParsedChanges { headers, entries })
}

fn parse_header<'a>(
    header: &'a str,
    context: &mut ParsedRepositoryContext,
    seen: &mut BTreeSet<&'a str>,
) -> Result<(), ErrorCode> {
    let (key, value) = header.split_once(' ').ok_or(ErrorCode::NotReady)?;
    if !seen.insert(key) {
        return Err(ErrorCode::NotReady);
    }
    match key {
        "branch.head" => match value {
            "(detached)" => context.detached = true,
            value if valid_display(value) => context.branch = Some(value.to_owned()),
            _ => return Err(ErrorCode::NotReady),
        },
        "branch.oid" => match value {
            "(initial)" => context.unborn = true,
            value if valid_context_oid(value) => context.head = Some(value.to_owned()),
            _ => return Err(ErrorCode::NotReady),
        },
        "branch.upstream" => {
            if !valid_display(value) {
                return Err(ErrorCode::NotReady);
            }
            context.upstream = Some(value.to_owned());
        }
        "branch.ab" => {
            let (ahead, behind) = value.split_once(' ').ok_or(ErrorCode::NotReady)?;
            let ahead = ahead.strip_prefix('+').ok_or(ErrorCode::NotReady)?;
            let behind = behind.strip_prefix('-').ok_or(ErrorCode::NotReady)?;
            if ahead == "?" && behind == "?" {
                context.ahead = None;
                context.behind = None;
            } else {
                context.ahead = Some(parse_context_count(ahead)?);
                context.behind = Some(parse_context_count(behind)?);
            }
        }
        _ => return Err(ErrorCode::NotReady),
    }
    Ok(())
}
fn parse_context_count(value: &str) -> Result<u64, ErrorCode> {
    if value.is_empty()
        || value.len() > 16
        || value.len() > 1 && value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ErrorCode::NotReady);
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or(ErrorCode::NotReady)
}

fn parse_ordinary(record: &str) -> Result<ParsedEntry, ErrorCode> {
    let mut parts = record.splitn(8, ' ');
    let xy = parts.next().ok_or(ErrorCode::NotReady)?;
    let submodule = parts.next().ok_or(ErrorCode::NotReady)?;
    let mode_head = parts.next().ok_or(ErrorCode::NotReady)?;
    let mode_index = parts.next().ok_or(ErrorCode::NotReady)?;
    let mode_worktree = parts.next().ok_or(ErrorCode::NotReady)?;
    let hash_head = parts.next().ok_or(ErrorCode::NotReady)?;
    let hash_index = parts.next().ok_or(ErrorCode::NotReady)?;
    let path = parts.next().ok_or(ErrorCode::NotReady)?;
    let (index, worktree) = parse_states(xy)?;
    validate_mode(mode_head)?;
    validate_mode(mode_index)?;
    validate_mode(mode_worktree)?;
    validate_hash(hash_head)?;
    validate_hash(hash_index)?;
    if renamed_or_copied(index) || renamed_or_copied(worktree) {
        return Err(ErrorCode::NotReady);
    }
    validate_path(path)?;
    Ok(ParsedEntry::Tracked(ParsedTracked {
        path: path.to_owned(),
        original_path: None,
        index,
        worktree,
        submodule: parse_submodule(submodule)?,
        staged_binary: false,
        unstaged_binary: false,
        staged_numstat_observed: false,
        unstaged_numstat_observed: false,
        staged_conflict_sentinel: false,
        conflict_stages: None,
    }))
}

fn parse_rename(record: &str, original_path: &str) -> Result<ParsedEntry, ErrorCode> {
    let mut parts = record.splitn(9, ' ');
    let xy = parts.next().ok_or(ErrorCode::NotReady)?;
    let submodule = parts.next().ok_or(ErrorCode::NotReady)?;
    let mode_head = parts.next().ok_or(ErrorCode::NotReady)?;
    let mode_index = parts.next().ok_or(ErrorCode::NotReady)?;
    let mode_worktree = parts.next().ok_or(ErrorCode::NotReady)?;
    let hash_head = parts.next().ok_or(ErrorCode::NotReady)?;
    let hash_index = parts.next().ok_or(ErrorCode::NotReady)?;
    let score = parts.next().ok_or(ErrorCode::NotReady)?;
    let path = parts.next().ok_or(ErrorCode::NotReady)?;
    let (index, worktree) = parse_states(xy)?;
    validate_mode(mode_head)?;
    validate_mode(mode_index)?;
    validate_mode(mode_worktree)?;
    validate_hash(hash_head)?;
    validate_hash(hash_index)?;
    let score_state = parse_score(score)?;
    if !renamed_or_copied(index) && !renamed_or_copied(worktree) {
        return Err(ErrorCode::NotReady);
    }
    if score_state != index && score_state != worktree {
        return Err(ErrorCode::NotReady);
    }
    validate_path(path)?;
    validate_path(original_path)?;
    Ok(ParsedEntry::Tracked(ParsedTracked {
        path: path.to_owned(),
        original_path: Some(original_path.to_owned()),
        index,
        worktree,
        submodule: parse_submodule(submodule)?,
        staged_binary: false,
        unstaged_binary: false,
        staged_numstat_observed: false,
        unstaged_numstat_observed: false,
        staged_conflict_sentinel: false,
        conflict_stages: None,
    }))
}

fn parse_unmerged(record: &str) -> Result<ParsedEntry, ErrorCode> {
    let mut parts = record.splitn(10, ' ');
    let xy = parts.next().ok_or(ErrorCode::NotReady)?;
    let submodule = parts.next().ok_or(ErrorCode::NotReady)?;
    let mode_stage1 = parts.next().ok_or(ErrorCode::NotReady)?;
    let mode_stage2 = parts.next().ok_or(ErrorCode::NotReady)?;
    let mode_stage3 = parts.next().ok_or(ErrorCode::NotReady)?;
    let mode_worktree = parts.next().ok_or(ErrorCode::NotReady)?;
    let hash_stage1 = parts.next().ok_or(ErrorCode::NotReady)?;
    let hash_stage2 = parts.next().ok_or(ErrorCode::NotReady)?;
    let hash_stage3 = parts.next().ok_or(ErrorCode::NotReady)?;
    let path = parts.next().ok_or(ErrorCode::NotReady)?;
    if !matches!(xy, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU") {
        return Err(ErrorCode::NotReady);
    }
    for mode in [mode_stage1, mode_stage2, mode_stage3, mode_worktree] {
        validate_mode(mode)?;
    }
    for hash in [hash_stage1, hash_stage2, hash_stage3] {
        validate_hash(hash)?;
    }
    let mut stages = 0_u8;
    for (bit, mode, hash) in [
        (1, mode_stage1, hash_stage1),
        (2, mode_stage2, hash_stage2),
        (4, mode_stage3, hash_stage3),
    ] {
        let present = mode != "000000";
        if present == hash.bytes().all(|byte| byte == b'0') {
            return Err(ErrorCode::NotReady);
        }
        if present {
            stages |= bit;
        }
    }
    let expected_stages = match xy {
        "DD" => 1,
        "AU" => 2,
        "UD" => 3,
        "UA" => 4,
        "DU" => 5,
        "AA" => 6,
        "UU" => 7,
        _ => return Err(ErrorCode::NotReady),
    };
    if stages != expected_stages {
        return Err(ErrorCode::NotReady);
    }
    validate_path(path)?;
    Ok(ParsedEntry::Tracked(ParsedTracked {
        path: path.to_owned(),
        original_path: None,
        index: ChangeState::Unmerged,
        worktree: ChangeState::Unmerged,
        submodule: parse_submodule(submodule)?,
        staged_binary: false,
        unstaged_binary: false,
        staged_numstat_observed: false,
        unstaged_numstat_observed: false,
        staged_conflict_sentinel: false,
        conflict_stages: Some(stages),
    }))
}

fn parse_states(value: &str) -> Result<(ChangeState, ChangeState), ErrorCode> {
    let bytes = value.as_bytes();
    if bytes.len() != 2 {
        return Err(ErrorCode::NotReady);
    }
    Ok((parse_state(bytes[0])?, parse_state(bytes[1])?))
}

fn parse_state(value: u8) -> Result<ChangeState, ErrorCode> {
    match value {
        b'.' => Ok(ChangeState::Unchanged),
        b'A' => Ok(ChangeState::Added),
        b'M' => Ok(ChangeState::Modified),
        b'D' => Ok(ChangeState::Deleted),
        b'R' => Ok(ChangeState::Renamed),
        b'C' => Ok(ChangeState::Copied),
        b'T' => Ok(ChangeState::TypeChanged),
        b'U' => Ok(ChangeState::Unmerged),
        _ => Err(ErrorCode::NotReady),
    }
}

fn parse_score(value: &str) -> Result<ChangeState, ErrorCode> {
    if value.len() < 2 || !value.is_char_boundary(1) {
        return Err(ErrorCode::NotReady);
    }
    let (kind, score) = value.split_at(1);
    let state = match kind {
        "R" => ChangeState::Renamed,
        "C" => ChangeState::Copied,
        _ => return Err(ErrorCode::NotReady),
    };
    if score.is_empty()
        || score.len() > 3
        || !score.bytes().all(|byte| byte.is_ascii_digit())
        || score.parse::<u8>().map_or(true, |value| value > 100)
    {
        return Err(ErrorCode::NotReady);
    }
    Ok(state)
}

fn validate_mode(value: &str) -> Result<(), ErrorCode> {
    if value.len() != 6 || !value.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
        return Err(ErrorCode::NotReady);
    }
    Ok(())
}

fn validate_hash(value: &str) -> Result<(), ErrorCode> {
    if !(value.len() == 40 || value.len() == 64)
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ErrorCode::NotReady);
    }
    Ok(())
}

fn parse_submodule(value: &str) -> Result<Option<ChangeSubmodule>, ErrorCode> {
    let bytes = value.as_bytes();
    if bytes == b"N..." {
        return Ok(None);
    }
    if bytes.len() != 4 || bytes[0] != b'S' {
        return Err(ErrorCode::NotReady);
    }
    let flag = |value: u8, enabled: u8| match value {
        b'.' => Ok(false),
        value if value == enabled => Ok(true),
        _ => Err(ErrorCode::NotReady),
    };
    Ok(Some(ChangeSubmodule {
        commit_changed: flag(bytes[1], b'C')?,
        tracked_dirty: flag(bytes[2], b'M')?,
        untracked_dirty: flag(bytes[3], b'U')?,
    }))
}

fn renamed_or_copied(state: ChangeState) -> bool {
    matches!(state, ChangeState::Renamed | ChangeState::Copied)
}

pub(super) struct NumstatEntry {
    path: String,
    original_path: Option<String>,
    counts: NumstatCounts,
}

pub(super) enum NumstatCounts {
    Text { added: u64, removed: u64 },
    Binary,
}

impl NumstatCounts {
    fn is_binary(&self) -> bool {
        matches!(self, Self::Binary)
    }

    fn is_unmerged_sentinel(&self) -> bool {
        matches!(
            self,
            Self::Text {
                added: 0,
                removed: 0
            }
        )
    }
}

pub(super) fn parse_numstat(
    bytes: &[u8],
    deadline: Instant,
) -> Result<Vec<NumstatEntry>, ErrorCode> {
    ensure_before(deadline)?;
    let output = std::str::from_utf8(bytes).map_err(|_| ErrorCode::NotReady)?;
    if !bytes.is_empty() && bytes.last() != Some(&0) {
        return Err(ErrorCode::NotReady);
    }
    let mut fields = output.split_terminator('\0');
    let mut entries = Vec::new();
    while let Some(field) = fields.next() {
        ensure_before(deadline)?;
        if field.is_empty() {
            return Err(ErrorCode::NotReady);
        }
        let mut parts = field.splitn(3, '\t');
        let added = parts.next().ok_or(ErrorCode::NotReady)?;
        let removed = parts.next().ok_or(ErrorCode::NotReady)?;
        let suffix = parts.next().ok_or(ErrorCode::NotReady)?;
        let counts = parse_numstat_counts(added, removed)?;
        let (path, original_path) = if suffix.is_empty() {
            let original_path = fields.next().ok_or(ErrorCode::NotReady)?;
            let path = fields.next().ok_or(ErrorCode::NotReady)?;
            validate_path(path)?;
            validate_path(original_path)?;
            (path.to_owned(), Some(original_path.to_owned()))
        } else {
            validate_path(suffix)?;
            (suffix.to_owned(), None)
        };
        // Each unmerged path may contribute a sentinel and one comparison.
        if entries.len() == MAX_ENTRIES * 2 {
            return Err(ErrorCode::ResourceLimit);
        }
        entries.push(NumstatEntry {
            path,
            original_path,
            counts,
        });
    }
    Ok(entries)
}

fn parse_numstat_counts(added: &str, removed: &str) -> Result<NumstatCounts, ErrorCode> {
    if added == "-" && removed == "-" {
        return Ok(NumstatCounts::Binary);
    }
    if added == "-" || removed == "-" {
        return Err(ErrorCode::NotReady);
    }
    if added.is_empty()
        || removed.is_empty()
        || !added.bytes().all(|byte| byte.is_ascii_digit())
        || !removed.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ErrorCode::NotReady);
    }
    let added = added.parse::<u64>().map_err(|_| ErrorCode::NotReady)?;
    let removed = removed.parse::<u64>().map_err(|_| ErrorCode::NotReady)?;
    Ok(NumstatCounts::Text { added, removed })
}

pub(super) fn apply_numstat(
    entries: &mut [ParsedEntry],
    observations: Vec<NumstatEntry>,
    staged: bool,
) -> Result<(), ErrorCode> {
    let mut applied = vec![0_u8; entries.len()];
    for observation in observations {
        let mut matched = None;
        for (index, entry) in entries.iter().enumerate() {
            let ParsedEntry::Tracked(tracked) = entry else {
                continue;
            };
            let state = if staged {
                tracked.index
            } else {
                tracked.worktree
            };
            let expected_original = renamed_or_copied(state)
                .then_some(tracked.original_path.as_deref())
                .flatten();
            let expected_path = tracked.path.as_str();
            if expected_path == observation.path
                && expected_original == observation.original_path.as_deref()
                && matched.replace(index).is_some()
            {
                return Err(ErrorCode::NotReady);
            }
        }
        let Some(index) = matched else {
            return Err(ErrorCode::NotReady);
        };
        let ParsedEntry::Tracked(tracked) = &mut entries[index] else {
            return Err(ErrorCode::NotReady);
        };
        let state = if staged {
            tracked.index
        } else {
            tracked.worktree
        };
        if state == ChangeState::Unmerged {
            let stages = tracked.conflict_stages.ok_or(ErrorCode::NotReady)?;
            match applied[index] {
                0 if observation.original_path.is_none()
                    && observation.counts.is_unmerged_sentinel() =>
                {
                    // A sentinel is not a comparison and proves no binary marker.
                    if staged {
                        tracked.staged_conflict_sentinel = true;
                        tracked.staged_binary = false;
                    } else {
                        tracked.unstaged_binary = false;
                    }
                }
                1 if !staged && stages & 2 != 0 && observation.original_path.is_none() => {
                    tracked.unstaged_binary = observation.counts.is_binary();
                }
                _ => return Err(ErrorCode::NotReady),
            }
        } else {
            if applied[index] != 0 {
                return Err(ErrorCode::NotReady);
            }
            if staged {
                tracked.staged_binary = observation.counts.is_binary();
            } else {
                tracked.unstaged_binary = observation.counts.is_binary();
            }
        }
        applied[index] += 1;
        if staged {
            tracked.staged_numstat_observed = true;
        } else {
            tracked.unstaged_numstat_observed = true;
        }
    }
    Ok(())
}

pub(super) fn require_numstat_completeness(entries: &[ParsedEntry]) -> Result<(), ErrorCode> {
    for entry in entries {
        let ParsedEntry::Tracked(tracked) = entry else {
            continue;
        };
        if tracked.conflict_stages.is_some()
            && (!tracked.staged_numstat_observed
                || !tracked.staged_conflict_sentinel
                || !tracked.unstaged_numstat_observed)
        {
            return Err(ErrorCode::NotReady);
        }
        if tracked.conflict_stages.is_none()
            && (tracked.index != ChangeState::Unchanged && !tracked.staged_numstat_observed
                || tracked.worktree != ChangeState::Unchanged && !tracked.unstaged_numstat_observed)
        {
            return Err(ErrorCode::NotReady);
        }
    }
    Ok(())
}

pub(super) fn parse_patch(bytes: &[u8], deadline: Instant) -> Result<String, ErrorCode> {
    ensure_before(deadline)?;
    if bytes.len() > MAX_PATCH_BYTES {
        return Err(ErrorCode::ResourceLimit);
    }
    let patch = std::str::from_utf8(bytes).map_err(|_| ErrorCode::NotReady)?;
    if patch
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(ErrorCode::NotReady);
    }
    Ok(patch.to_owned())
}

pub(super) fn validate_path(value: &str) -> Result<(), ErrorCode> {
    if value.len() > MAX_PATH_BYTES {
        return Err(ErrorCode::ResourceLimit);
    }
    if value.is_empty()
        || value.chars().any(char::is_control)
        || !Path::new(value).is_relative()
        || Path::new(value).components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(ErrorCode::NotReady);
    }
    Ok(())
}

fn valid_display(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_DISPLAY_BYTES && !value.chars().any(char::is_control)
}

fn ensure_before(deadline: Instant) -> Result<(), ErrorCode> {
    (Instant::now() < deadline)
        .then_some(())
        .ok_or(ErrorCode::Timeout)
}

pub(super) fn parse_gitlinks(bytes: &[u8], deadline: Instant) -> Result<Vec<String>, ErrorCode> {
    ensure_before(deadline)?;
    let output = std::str::from_utf8(bytes).map_err(|_| ErrorCode::NotReady)?;
    if !bytes.is_empty() && bytes.last() != Some(&0) {
        return Err(ErrorCode::NotReady);
    }
    let mut paths = BTreeSet::new();
    for record in output.split_terminator('\0') {
        if record.is_empty() {
            return Err(ErrorCode::NotReady);
        }
        ensure_before(deadline)?;
        let (metadata, path) = record.split_once('\t').ok_or(ErrorCode::NotReady)?;
        let mut fields = metadata.split(' ');
        let mode = fields.next().ok_or(ErrorCode::NotReady)?;
        let hash = fields.next().ok_or(ErrorCode::NotReady)?;
        let stage = fields.next().ok_or(ErrorCode::NotReady)?;
        if fields.next().is_some() {
            return Err(ErrorCode::NotReady);
        }
        validate_mode(mode)?;
        validate_hash(hash)?;
        if !matches!(stage, "0" | "1" | "2" | "3") {
            return Err(ErrorCode::NotReady);
        }
        if mode == "160000" && stage == "0" {
            validate_path(path)?;
            if paths.len() >= MAX_ENTRIES {
                return Err(ErrorCode::ResourceLimit);
            }
            if !paths.insert(path.to_owned()) {
                return Err(ErrorCode::NotReady);
            }
        }
    }
    Ok(paths.into_iter().collect())
}

#[derive(Default)]
struct Worktree {
    path: Option<String>,
    branch: Option<String>,
    head: Option<String>,
    bare: bool,
    detached: bool,
    locked: bool,
    prunable: bool,
}
impl Worktree {
    fn push(&mut self, field: &str) -> SecurityResult<()> {
        if self.path.is_none() && !field.starts_with("worktree ") {
            return Err(ErrorCode::NotReady);
        }
        if let Some(path) = field.strip_prefix("worktree ") {
            if self.path.is_some()
                || !Path::new(path).is_absolute()
                || path.is_empty()
                || path.len() > MAX_PATH_BYTES
                || path.chars().any(char::is_control)
            {
                return Err(ErrorCode::NotReady);
            }
            self.path = Some(path.to_owned());
        } else if let Some(head) = field.strip_prefix("HEAD ") {
            if self.head.is_some() || !valid_context_oid(head) {
                return Err(ErrorCode::NotReady);
            }
            self.head = Some(head.to_owned());
        } else if let Some(branch) = field.strip_prefix("branch ") {
            let branch = branch
                .strip_prefix("refs/heads/")
                .ok_or(ErrorCode::NotReady)?;
            if self.branch.is_some() || !valid_display(branch) {
                return Err(ErrorCode::NotReady);
            }
            self.branch = Some(branch.to_owned());
        } else if field == "bare" {
            if self.bare {
                return Err(ErrorCode::NotReady);
            }
            self.bare = true;
        } else if field == "detached" {
            if self.detached {
                return Err(ErrorCode::NotReady);
            }
            self.detached = true;
        } else if field == "locked" || field.starts_with("locked ") {
            if self.locked {
                return Err(ErrorCode::NotReady);
            }
            self.locked = true;
        } else if field == "prunable" || field.starts_with("prunable ") {
            if self.prunable {
                return Err(ErrorCode::NotReady);
            }
            self.prunable = true;
        } else {
            return Err(ErrorCode::NotReady);
        }
        Ok(())
    }
    fn finish(self) -> SecurityResult<WorktreeEntry> {
        let path = self.path.ok_or(ErrorCode::NotReady)?;
        if self.bare && (self.branch.is_some() || self.head.is_some() || self.detached)
            || self.detached && (self.branch.is_some() || self.head.is_none())
            || !self.bare && !self.detached && self.branch.is_none()
        {
            return Err(ErrorCode::NotReady);
        }
        let label = Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| valid_display(name))
            .ok_or(ErrorCode::NotReady)?
            .to_owned();
        Ok(WorktreeEntry {
            path,
            label,
            branch: self.branch,
            head: self.head,
            is_bare: self.bare,
            is_detached: self.detached,
            is_locked: self.locked,
            is_prunable: self.prunable,
            is_linked_worktree: None,
        })
    }
}
pub(super) fn parse_worktrees(
    bytes: &[u8],
    deadline: Instant,
) -> SecurityResult<Vec<WorktreeEntry>> {
    ensure_before(deadline)?;
    if bytes.last() != Some(&0) {
        return Err(ErrorCode::NotReady);
    }
    let output = std::str::from_utf8(bytes).map_err(|_| ErrorCode::NotReady)?;
    let mut current = Worktree::default();
    let mut entries = Vec::new();
    for field in output.split_terminator('\0') {
        ensure_before(deadline)?;
        if field.is_empty() {
            if current.path.is_some() {
                let entry = std::mem::take(&mut current).finish()?;
                if entries.len() == MAX_ENTRIES {
                    return Err(ErrorCode::ResourceLimit);
                }
                entries.push(entry);
            }
        } else {
            if field.starts_with("worktree ") && current.path.is_some() {
                return Err(ErrorCode::NotReady);
            }
            current.push(field)?;
        }
    }
    if current.path.is_some() {
        let entry = current.finish()?;
        if entries.len() == MAX_ENTRIES {
            return Err(ErrorCode::ResourceLimit);
        }
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(ErrorCode::NotReady);
    }
    {
        let mut paths = BTreeSet::new();
        for entry in &entries {
            if !paths.insert(entry.path.as_str()) {
                return Err(ErrorCode::NotReady);
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }
    #[test]
    fn actual_porcelain_rename_joins_both_native_paths_and_staged_numstat() {
        let oid = "1".repeat(40);
        let status = format!(
            "# branch.oid {oid}\0# branch.head main\02 R. N... 100644 100644 100644 {oid} {oid} R100 new-name\0old-name\0? untracked\0"
        );
        let mut parsed = parse_status(status.as_bytes(), deadline()).unwrap();
        apply_numstat(
            &mut parsed.entries,
            parse_numstat(b"0\t0\t\0old-name\0new-name\0", deadline()).unwrap(),
            true,
        )
        .unwrap();
        require_numstat_completeness(&parsed.entries).unwrap();
        let ParsedEntry::Tracked(row) = &parsed.entries[0] else {
            panic!("native tracked rename absent")
        };
        assert_eq!(row.path, "new-name");
        assert_eq!(row.original_path.as_deref(), Some("old-name"));
        assert_eq!(row.index, ChangeState::Renamed);
        assert_eq!(parsed.headers.head.as_deref(), Some(oid.as_str()));
        assert!(!parsed.headers.unborn);
        let end = status.find("new-name\0").unwrap() + "new-name\0".len();
        assert!(parse_status(&status.as_bytes()[..end], deadline()).is_err());
    }
    #[test]
    fn binary_facts_require_both_native_sides_instead_of_defaulting_to_text() {
        let oid = "2".repeat(40);
        let status = format!(
            "# branch.oid {oid}\0# branch.head main\01 MM N... 100644 100644 100644 {oid} {oid} binary\0"
        );
        let mut parsed = parse_status(status.as_bytes(), deadline()).unwrap();
        assert!(require_numstat_completeness(&parsed.entries).is_err());
        apply_numstat(
            &mut parsed.entries,
            parse_numstat(b"-\t-\tbinary\0", deadline()).unwrap(),
            true,
        )
        .unwrap();
        assert!(require_numstat_completeness(&parsed.entries).is_err());
        apply_numstat(
            &mut parsed.entries,
            parse_numstat(b"-\t-\tbinary\0", deadline()).unwrap(),
            false,
        )
        .unwrap();
        require_numstat_completeness(&parsed.entries).unwrap();
        let ParsedEntry::Tracked(row) = &parsed.entries[0] else {
            panic!("binary entry absent")
        };
        assert!(row.staged_binary && row.unstaged_binary);
        assert!(parse_numstat(b"-\t1\tbinary\0", deadline()).is_err());
        assert!(parse_numstat(b"1\t1\tbinary", deadline()).is_err());
    }
    #[test]
    fn complete_worktree_rows_reject_foreign_fields_duplicates_and_malformed_prefixes() {
        let oid = "3".repeat(40);
        let valid = format!(
            "worktree /approved/root\0HEAD {oid}\0branch refs/heads/main\0\0worktree /approved/linked\0HEAD {oid}\0detached\0locked native reason\0\0"
        );
        let rows = parse_worktrees(valid.as_bytes(), deadline()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].branch.as_deref(), Some("main"));
        assert!(rows[1].is_locked && rows[1].is_detached);
        assert_eq!(rows[1].is_linked_worktree, None);
        for malformed in [
            format!("bare\0{valid}"),
            format!("{valid}{valid}"),
            valid.replace("HEAD ", "unknown "),
            valid.trim_end_matches('\0').to_owned(),
        ] {
            assert!(parse_worktrees(malformed.as_bytes(), deadline()).is_err());
        }
    }
    #[test]
    fn native_context_and_patch_bounds_refuse_ambiguous_or_partial_records() {
        let unborn =
            parse_status(b"# branch.oid (initial)\0# branch.head main\0", deadline()).unwrap();
        assert!(unborn.headers.unborn);
        assert!(unborn.headers.head.is_none());
        assert!(
            parse_status(
                b"# branch.oid (initial)\0# branch.head main\0# branch.head another\0",
                deadline()
            )
            .is_err()
        );
        assert!(parse_patch(b"partial\0patch", deadline()).is_err());
        assert!(parse_patch(&vec![b'x'; MAX_PATCH_BYTES + 1], deadline()).is_err());
        assert!(parse_latest_commit(b"bad\0subject\0", &"4".repeat(40), deadline()).is_err());
    }
}
