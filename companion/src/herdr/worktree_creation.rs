//! Read-only durable checkout facts retained for historical creation receipts, never grants.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeObjectProof {
    pub path: String,
    pub device: Option<u64>,
    pub inode: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeSourceProof {
    pub cwd: String,
    pub repo_root: NativeObjectProof,
    pub git_dir: NativeObjectProof,
    pub common_git_dir: NativeObjectProof,
    pub is_bare: bool,
}
/// Historical host-only facts. Decoding these never authorizes a new native operation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionCheckout {
    pub action_id: Uuid,
    pub source_target: Target,
    pub source_epoch: Uuid,
    pub server_id: Uuid,
    pub server_incarnation: Uuid,
    pub evidence: ServerEvidence,
    pub scope: WorktreeReadScope,
    pub project_id: Uuid,
    pub project_object: (u64, u64),
    pub source: NativeSourceProof,
    pub checkout: CreationCheckout,
    pub resolved_commit: Option<String>,
    pub path: Option<String>,
    pub object: Option<NativeObjectProof>,
    pub checkout_source: Option<NativeSourceProof>,
    pub tab_id: Option<Id>,
}
impl ExecutionCheckout {
    pub(crate) fn retains(&self, old: &Self) -> bool {
        self.action_id == old.action_id
            && self.source_target == old.source_target
            && self.source_epoch == old.source_epoch
            && self.server_id == old.server_id
            && self.server_incarnation == old.server_incarnation
            && self.evidence == old.evidence
            && self.scope == old.scope
            && self.project_id == old.project_id
            && self.project_object == old.project_object
            && self.source == old.source
            && self.checkout == old.checkout
            && old
                .resolved_commit
                .as_ref()
                .is_none_or(|value| self.resolved_commit.as_ref() == Some(value))
            && old
                .path
                .as_ref()
                .is_none_or(|value| self.path.as_ref() == Some(value))
            && old
                .object
                .as_ref()
                .is_none_or(|value| self.object.as_ref() == Some(value))
            && old
                .checkout_source
                .as_ref()
                .is_none_or(|value| self.checkout_source.as_ref() == Some(value))
            && old
                .tab_id
                .as_ref()
                .is_none_or(|value| self.tab_id.as_ref() == Some(value))
    }
}
