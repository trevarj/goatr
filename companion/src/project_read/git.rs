// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded reference/review observations from the original authenticated project.
use super::*;
use crate::{herdr::artifacts::ReadJob, pairing::SecurityResult, protocol::*, state::State};
use std::{sync::Arc, time::Duration};
mod admin;
mod guard;
mod parse;
mod runtime;
const MAX_OUTPUT: usize = 256 * 1024;
// Configuration observation can contain credentials. Its raw stdout and parsed
// records have no lifetime beyond this worker, including error/reallocation paths.
fn erase(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}
struct NativeOutput(Vec<u8>);
impl NativeOutput {
    fn as_slice(&self) -> &[u8] {
        &self.0
    }
    fn append(&mut self, bytes: &[u8]) -> SecurityResult<()> {
        let required = self
            .0
            .len()
            .checked_add(bytes.len())
            .filter(|length| *length <= MAX_OUTPUT)
            .ok_or(ErrorCode::ResourceLimit)?;
        if required > self.0.capacity() {
            let capacity = required.next_power_of_two().min(MAX_OUTPUT);
            let mut replacement = Vec::new();
            replacement
                .try_reserve_exact(capacity)
                .map_err(|_| ErrorCode::ResourceLimit)?;
            replacement.extend_from_slice(&self.0);
            erase(&mut self.0);
            self.0 = replacement;
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}
impl std::ops::Deref for NativeOutput {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for NativeOutput {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}

const STATUS: &[&str] = &[
    "status",
    "--porcelain=v2",
    "-z",
    "--branch",
    "--ahead-behind",
    "--untracked-files=all",
    "--ignore-submodules=dirty",
    "--renames",
];
fn ensure_before(deadline: Instant) -> SecurityResult<()> {
    if Instant::now() >= deadline {
        Err(ErrorCode::Timeout)
    } else {
        Ok(())
    }
}
pub(crate) fn available() -> bool {
    guard::supported() && runtime::selected(Instant::now() + Duration::from_secs(5)).is_ok()
}
struct Observation {
    graph: admin::Graph,
    runtime: &'static runtime::Runtime,
    job: Arc<ReadJob>,
    deadline: Instant,
    used: usize,
    repositories: Vec<admin::Repository>,
}
impl Observation {
    fn new(
        context: ProjectReadContext,
        job: Arc<ReadJob>,
        deadline: Instant,
    ) -> SecurityResult<Self> {
        ensure_before(deadline)?;
        job.current()?;
        let runtime = runtime::selected(deadline)?;
        let mut graph = admin::Graph::new(context, &job, runtime, deadline)?;
        graph.seed_configuration(runtime, deadline)?;
        Ok(Self {
            graph,
            runtime,
            job,
            deadline,
            used: 0,
            repositories: Vec::new(),
        })
    }
    fn output(&mut self, cwd: &Path, operation: &[&str]) -> SecurityResult<NativeOutput> {
        ensure_before(self.deadline)?;
        self.graph.directory(cwd, self.deadline)?;
        let original = self.graph.context.location().clone();
        let (status, output) = guard::run(self, &original.cwd, cwd, operation)?;
        if status != 0 {
            return Err(ErrorCode::NotReady);
        }
        Ok(output)
    }
    fn discover(&mut self, cwd: &Path, exact: bool) -> SecurityResult<admin::Repository> {
        let candidate = self.graph.candidate(cwd, exact, self.deadline)?;
        let bare = self.output(cwd, &["rev-parse", "--is-bare-repository"])?;
        let bare = match bare.as_slice() {
            b"true\n" => true,
            b"false\n" => false,
            _ => return Err(ErrorCode::NotReady),
        };
        let git = line_path(&self.output(cwd, &["rev-parse", "--absolute-git-dir"])?)?;
        let common = line_path(&self.output(
            cwd,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?)?;
        let root = if bare {
            git.clone()
        } else {
            line_path(&self.output(
                cwd,
                &["rev-parse", "--path-format=absolute", "--show-toplevel"],
            )?)?
        };
        if root != candidate.root
            || git != candidate.git
            || common != candidate.common
            || exact && root != cwd
        {
            return Err(ErrorCode::NotReady);
        }
        let repo = admin::Repository {
            root,
            git,
            common,
            bare,
        };
        let names = self.output(
            cwd,
            &[
                "config",
                "--null",
                "--show-scope",
                "--show-origin",
                "--name-only",
                "--list",
            ],
        )?;
        let values = self.output(
            cwd,
            &[
                "config",
                "--null",
                "--show-scope",
                "--show-origin",
                "--list",
            ],
        )?;
        let records = admin::config_records(&names, &values)?;
        self.graph
            .validate_config(&records, &repo, self.runtime, self.deadline)?;
        Ok(repo)
    }
    fn root(&mut self) -> SecurityResult<admin::Repository> {
        let cwd = self.graph.context.location().cwd_path().to_owned();
        let repo = self.discover(&cwd, false)?;
        self.repositories.push(repo.clone());
        self.graph.control_reads = false;
        Ok(repo)
    }
    fn worktrees(&mut self, target: Target) -> SecurityResult<ResultBody> {
        let repo = self.root()?;
        let bytes = self.output(&repo.root, &["worktree", "list", "--porcelain", "-z"])?;
        let mut entries = parse::parse_worktrees(&bytes, self.deadline)?;
        for entry in &mut entries {
            entry.is_linked_worktree = self.graph.validate_worktree(
                Path::new(&entry.path),
                entry.is_prunable,
                self.deadline,
            )?;
        }
        let cwd = self.graph.context.location().cwd_path().to_owned();
        if self.discover(&cwd, false)? != repo {
            return Err(ErrorCode::StaleSession);
        }
        let result = WorktreesListResult {
            target,
            worktrees: entries,
        };
        result.validate().map_err(|_| ErrorCode::NotReady)?;
        Ok(ResultBody::WorktreesList(result))
    }
    fn changes(&mut self, target: Target) -> SecurityResult<ResultBody> {
        let repo = self.root()?;
        if repo.bare {
            return Err(ErrorCode::NotReady);
        }
        let status = self.output(&repo.root, STATUS)?;
        let mut parsed = parse::parse_status(&status, self.deadline)?;
        let staged = self.output(
            &repo.root,
            &[
                "diff",
                "--numstat",
                "-z",
                "--no-ext-diff",
                "--no-textconv",
                "--submodule=short",
                "--ignore-submodules=dirty",
                "--find-renames",
                "--cached",
            ],
        )?;
        parse::apply_numstat(
            &mut parsed.entries,
            parse::parse_numstat(&staged, self.deadline)?,
            true,
        )?;
        drop(staged);
        let unstaged = self.output(
            &repo.root,
            &[
                "diff",
                "--numstat",
                "-z",
                "--no-ext-diff",
                "--no-textconv",
                "--submodule=short",
                "--ignore-submodules=dirty",
                "--find-renames",
            ],
        )?;
        parse::apply_numstat(
            &mut parsed.entries,
            parse::parse_numstat(&unstaged, self.deadline)?,
            false,
        )?;
        drop(unstaged);
        parse::require_numstat_completeness(&parsed.entries)?;
        let staged = self.output(
            &repo.root,
            &[
                "diff",
                "--patch",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--submodule=short",
                "--ignore-submodules=dirty",
                "--find-renames",
                "--cached",
            ],
        )?;
        let staged_patch = parse::parse_patch(&staged, self.deadline)?;
        drop(staged);
        let unstaged = self.output(
            &repo.root,
            &[
                "diff",
                "--patch",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--submodule=short",
                "--ignore-submodules=dirty",
                "--find-renames",
            ],
        )?;
        let unstaged_patch = parse::parse_patch(&unstaged, self.deadline)?;
        drop(unstaged);
        self.submodules(&repo, &mut parsed.entries)?;
        let latest_commit = if let Some(oid) = &parsed.headers.head {
            let bytes = self.output(
                &repo.root,
                &[
                    "show",
                    "--no-patch",
                    "--no-notes",
                    "--no-show-signature",
                    "--no-color",
                    "--encoding=UTF-8",
                    "--format=format:%H%x00%s%x00",
                    oid,
                    "--",
                ],
            )?;
            Some(parse::parse_latest_commit(&bytes, oid, self.deadline)?)
        } else {
            None
        };
        let after = self.output(&repo.root, STATUS)?;
        if parse::parse_status(&after, self.deadline)?.headers != parsed.headers {
            return Err(ErrorCode::StaleSession);
        }
        // Context and held admin objects are coherent; changed worktree contents
        // are native Git's ordinary observation, not an atomic filesystem snapshot.
        for index in 0..self.repositories.len() {
            let original = self.repositories[index].clone();
            if self.discover(&original.root, true)? != original {
                return Err(ErrorCode::StaleSession);
            }
        }
        let changes = ChangesInfo {
            repository_context: Some(ChangesRepositoryContext {
                unborn: parsed.headers.unborn,
                upstream: parsed.headers.upstream.map(|name| ChangesUpstream {
                    name,
                    ahead: parsed.headers.ahead.map(Uint),
                    behind: parsed.headers.behind.map(Uint),
                }),
                latest_commit,
            }),
            branch: parsed.headers.branch,
            head: parsed.headers.head,
            detached: parsed.headers.detached,
            entries: parsed
                .entries
                .into_iter()
                .map(parse::ParsedEntry::into_entry)
                .collect(),
            staged_patch,
            unstaged_patch,
        };
        let result = ChangesReadResult { target, changes };
        result.validate().map_err(|_| ErrorCode::NotReady)?;
        Ok(ResultBody::ChangesRead(result))
    }
    fn submodules(
        &mut self,
        repo: &admin::Repository,
        entries: &mut Vec<parse::ParsedEntry>,
    ) -> SecurityResult<()> {
        let inventory = self.output(&repo.root, &["ls-files", "--stage", "-z"])?;
        for path in parse::parse_gitlinks(&inventory, self.deadline)? {
            let checkout = repo.root.join(&path);
            if !self.graph.initialized_submodule(&checkout, self.deadline)? {
                continue;
            }
            if self.repositories.len() == 16 {
                return Err(ErrorCode::ResourceLimit);
            }
            let source = self.discover(&checkout, true)?;
            if source.bare
                || self
                    .repositories
                    .iter()
                    .any(|seen| seen.git == source.git && seen.common == source.common)
            {
                return Err(ErrorCode::NotReady);
            }
            self.repositories.push(source.clone());
            let bytes = self.output(&source.root, STATUS)?;
            let mut nested = parse::parse_status(&bytes, self.deadline)?;
            let base_dirty = nested
                .entries
                .iter()
                .any(|entry| matches!(entry, parse::ParsedEntry::Tracked(_)));
            self.submodules(&source, &mut nested.entries)?;
            let tracked_dirty=base_dirty||nested.entries.iter().any(|entry|matches!(entry,parse::ParsedEntry::Tracked(row) if row.submodule.as_ref().is_some_and(|submodule|submodule.commit_changed||submodule.tracked_dirty)));
            let untracked_dirty = nested.entries.iter().any(|entry| match entry {
                parse::ParsedEntry::Tracked(row) => row
                    .submodule
                    .as_ref()
                    .is_some_and(|submodule| submodule.untracked_dirty),
                parse::ParsedEntry::Untracked { .. } => true,
            });
            let existing = entries.iter_mut().find_map(|entry| match entry {
                parse::ParsedEntry::Tracked(row) if row.path == path => Some(row),
                _ => None,
            });
            if let Some(row) = existing {
                let prior = row.submodule.take().unwrap_or(ChangeSubmodule {
                    commit_changed: false,
                    tracked_dirty: false,
                    untracked_dirty: false,
                });
                row.submodule = Some(ChangeSubmodule {
                    commit_changed: prior.commit_changed,
                    tracked_dirty: prior.tracked_dirty || tracked_dirty,
                    untracked_dirty: prior.untracked_dirty || untracked_dirty,
                });
                if row.worktree == ChangeState::Unchanged && (tracked_dirty || untracked_dirty) {
                    row.worktree = ChangeState::Modified;
                }
            } else if tracked_dirty || untracked_dirty {
                if entries.len() == 128 || entries.iter().any(|entry| entry.path() == path) {
                    return Err(ErrorCode::ResourceLimit);
                }
                entries.push(parse::ParsedEntry::Tracked(parse::ParsedTracked {
                    path,
                    original_path: None,
                    index: ChangeState::Unchanged,
                    worktree: ChangeState::Modified,
                    submodule: Some(ChangeSubmodule {
                        commit_changed: false,
                        tracked_dirty,
                        untracked_dirty,
                    }),
                    staged_binary: false,
                    unstaged_binary: false,
                    staged_numstat_observed: false,
                    unstaged_numstat_observed: false,
                    staged_conflict_sentinel: false,
                    conflict_stages: None,
                }));
            }
        }
        Ok(())
    }
}
fn line_path(bytes: &[u8]) -> SecurityResult<PathBuf> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ErrorCode::NotReady)?
        .strip_suffix('\n')
        .ok_or(ErrorCode::NotReady)?;
    let path = admin::normalized(text, Path::new("/"))?;
    if !Path::new(text).is_absolute() || path.as_os_str() != OsStr::new(text) {
        return Err(ErrorCode::NotReady);
    }
    Ok(path)
}
pub(crate) struct Prepared {
    graph: admin::Graph,
    runtime: &'static runtime::Runtime,
    job: Arc<ReadJob>,
    deadline: Instant,
    result: Option<ResultBody>,
}
impl Prepared {
    pub(crate) fn enqueue(
        mut self,
        state: &State,
        connection: &Uuid,
        id: Uuid,
    ) -> SecurityResult<()> {
        let context = self.graph.context.clone();
        let target = match context.source() {
            ArtifactSource::Session { target } => target.clone(),
            _ => return Err(ErrorCode::NotReady),
        };
        let mut response_id = Some(id);
        context.with_current(self.deadline, || {
            ensure_before(self.deadline)?;
            self.job.current()?;
            self.runtime.current(self.deadline)?;
            self.graph.objects_current(self.deadline)?;
            ensure_before(self.deadline)?;
            let id = response_id.take().ok_or(ErrorCode::NotReady)?;
            match self.result.take().ok_or(ErrorCode::NotReady)? {
                ResultBody::WorktreesList(result) => {
                    state.worktrees_list_response(connection, &target, context.epoch(), id, result)
                }
                ResultBody::ChangesRead(result) => {
                    state.changes_read_response(connection, &target, context.epoch(), id, result)
                }
                _ => Err(ErrorCode::NotReady),
            }
        })
    }
}
pub(crate) async fn observe(
    context: ProjectReadContext,
    job: Arc<ReadJob>,
    changes: bool,
) -> SecurityResult<Prepared> {
    let deadline = context.observation_deadline().min(job.deadline);
    ensure_before(deadline)?;
    let target = match context.source() {
        ArtifactSource::Session { target } => target.clone(),
        _ => return Err(ErrorCode::NotReady),
    };
    let worker = tokio::task::spawn_blocking(move || {
        let mut observation = Observation::new(context, job, deadline)?;
        let result = if changes {
            observation.changes(target)?
        } else {
            observation.worktrees(target)?
        };
        observation.graph.recheck(deadline)?;
        Ok(Prepared {
            graph: observation.graph,
            runtime: observation.runtime,
            job: observation.job,
            deadline,
            result: Some(result),
        })
    });
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), worker)
        .await
        .map_err(|_| ErrorCode::Timeout)?
        .map_err(|_| ErrorCode::NotReady)?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_output_bound_refuses_growth_without_publishing_a_partial_extra_byte() {
        let mut output = NativeOutput(Vec::new());
        output.append(&vec![b'x'; MAX_OUTPUT]).unwrap();
        assert_eq!(output.append(b"private"), Err(ErrorCode::ResourceLimit));
        assert_eq!(output.len(), MAX_OUTPUT);
        assert!(output.iter().all(|byte| *byte == b'x'));
    }

    #[test]
    fn native_repository_paths_are_complete_absolute_unambiguous_records() {
        assert_eq!(
            line_path(b"/approved/repo\n").unwrap(),
            Path::new("/approved/repo")
        );
        for bytes in [
            b"relative\n".as_slice(),
            b"/approved/repo\nprivate\n",
            b"/approved/repo",
            b"/approved/../outside\n",
            b"/approved/repo\0\n",
        ] {
            assert!(line_path(bytes).is_err());
        }
    }
}
