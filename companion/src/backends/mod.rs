// SPDX-License-Identifier: GPL-3.0-or-later
pub mod codex;
pub mod omp;

use crate::{
    herdr::{self,ProcessEvidence,Source},
    pairing::SecurityResult,
    protocol::*,
};
use std::{collections::HashMap,path::Path,sync::{Arc,atomic::{AtomicUsize,Ordering}},time::Instant};

pub(crate) type PublicationGuard=Arc<dyn Fn(&mut dyn FnMut()->SecurityResult<()>)->SecurityResult<()>+Send+Sync>;

/// Original provider authority only. No attachment, terminal, document or controls grant is inherited.
#[derive(Clone)]
pub(crate) struct OriginalContext {
    pub(crate) provider:Provider,
    pub(crate) target:Target,
    pub(crate) epoch:Uuid,
    pub(crate) source:Source,
    pub(crate) process:ProcessEvidence,
    pub(crate) native:Id,
    validate:Arc<dyn Fn()->SecurityResult<()>+Send+Sync>,
    publish:PublicationGuard,
}
impl OriginalContext {
    pub(crate) fn capture(
        provider:Provider,target:Target,epoch:Uuid,source:Source,process:ProcessEvidence,native:Id,
        validate:Arc<dyn Fn()->SecurityResult<()>+Send+Sync>,publish:PublicationGuard,
    )->SecurityResult<Self> {
        let context=Self {provider,target,epoch,source,process,native,validate,publish};
        context.recheck()?;Ok(context)
    }
    pub(crate) fn guarded(
        mut self,check:Arc<dyn Fn()->SecurityResult<()>+Send+Sync>,fence:PublicationGuard,
    )->Self {
        let validate=self.validate;self.validate=Arc::new(move||{validate()?;check()});
        let publish=self.publish;self.publish=Arc::new(move|commit|publish(&mut ||fence(commit)));
        self
    }
    pub(crate) fn authorized(self,auth:&Arc<crate::pairing::Authorized>)->Self {
        let check=auth.clone();let fence=auth.clone();
        self.guarded(Arc::new(move||check.live_connection()),Arc::new(move|commit|{fence.live_connection()?;commit()}))
    }
    pub(crate) fn leased(self,expires:Instant)->Self {
        self.guarded(
            Arc::new(move||if Instant::now()<expires {Ok(())}else{Err(ErrorCode::StaleSession)}),
            Arc::new(move|commit|{if Instant::now()>=expires {return Err(ErrorCode::StaleSession)}commit()}),
        )
    }
    pub(crate) fn recheck(&self)->SecurityResult<()> {
        (self.validate)()?;self.process_current()
    }
    fn process_current(&self)->SecurityResult<()> {
        if herdr::process_evidence(self.process.pid).map_err(|_|ErrorCode::StaleSession)?!=self.process {
            return Err(ErrorCode::StaleSession);
        }
        Ok(())
    }
    pub(crate) fn file_visible(&self,pathname:&Path,object:(u64,u64))->SecurityResult<()> {
        self.recheck()?;
        if herdr::process_object(&self.process,pathname).map_err(|_|ErrorCode::NotReady)?!=object {
            return Err(ErrorCode::NotReady);
        }
        self.recheck()
    }
    pub(crate) fn with_current(&self,mut commit:impl FnMut()->SecurityResult<()>)->SecurityResult<()> {
        // The publication guard already owns membership; never recursively acquire it.
        (self.publish)(&mut ||{self.process_current()?;commit()})
    }
}

#[derive(Clone,Copy)]
pub(crate) enum PublicationKind {Skills,Controls,Queue}
/// Charged until the last real catalogue/job owner drops, not until its waiter disappears.
pub(crate) struct PublicationSlot {count:Arc<AtomicUsize>}
impl PublicationSlot {
    pub(crate) fn reserve(count:Arc<AtomicUsize>,limit:usize)->SecurityResult<Arc<Self>> {
        count.try_update(Ordering::AcqRel,Ordering::Acquire,|used|(used<limit).then_some(used+1))
            .map_err(|_|ErrorCode::ResourceLimit)?;
        Ok(Arc::new(Self {count}))
    }
}
impl Drop for PublicationSlot {
    fn drop(&mut self) {self.count.fetch_sub(1,Ordering::AcqRel);}
}
pub(crate) struct PublicationOrigin {
    pub(crate) peer:Uuid,
    pub(crate) peer_epoch:u64,
    pub(crate) connection:Uuid,
    pub(crate) context:OriginalContext,
}
pub(crate) struct SkillDocument {
    pub(crate) root:Arc<crate::project_read::Root>,
    pub(crate) file:Arc<crate::project_read::FileSelection>,
    pub(crate) native_reference:Option<Uuid>,
}
pub(crate) struct SkillsPublication {
    pub(crate) origin:PublicationOrigin,
    pub(crate) catalogue:SkillsListResult,
    pub(crate) documents:HashMap<Uuid,SkillDocument>,
    pub(crate) document_expires:Instant,
    pub(crate) native_catalogue:Option<Uuid>,
    pub(crate) native_cwd:Option<String>,
    pub(crate) slot:Arc<PublicationSlot>,
}
#[derive(Clone,PartialEq,Eq)]
pub(crate) struct NativeModelIdentity {
    pub(crate) provider:String,
    pub(crate) model:String,
}
#[derive(Clone,Default,PartialEq,Eq)]
pub(crate) struct EffectiveControls {
    pub(crate) model:Option<NativeModelIdentity>,
    pub(crate) effort:Option<Text>,
    pub(crate) mode:Option<ControlMode>,
}
pub(crate) struct ControlsObservation {
    pub(crate) current:EffectiveControls,
    pub(crate) generation:Uuid,
    changed:tokio::sync::watch::Sender<Uuid>,
}
impl ControlsObservation {
    pub(crate) fn new(current:EffectiveControls)->Self {
        let generation=Uuid::random();let (changed,_)=tokio::sync::watch::channel(generation.clone());
        Self {current,generation,changed}
    }
    pub(crate) fn update(&mut self,current:EffectiveControls)->bool {
        if self.current==current {return false}
        self.current=current;self.generation=Uuid::random();self.changed.send_replace(self.generation.clone());true
    }
    pub(crate) fn subscribe(&self)->tokio::sync::watch::Receiver<Uuid> {self.changed.subscribe()}
}
pub(crate) struct NativeControlModel {
    pub(crate) identity:NativeModelIdentity,
    pub(crate) native_reference:Option<Uuid>,
}
pub(crate) struct ControlsPublication {
    pub(crate) origin:PublicationOrigin,
    pub(crate) catalogue:SessionControlsReadResult,
    pub(crate) models:Vec<NativeControlModel>,
    pub(crate) current_native:Option<NativeModelIdentity>,
    pub(crate) native_catalogue:Option<Uuid>,
    /// Local observed-change token, never a fabricated native epoch.
    pub(crate) observation:Uuid,
    pub(crate) expires:Instant,
    pub(crate) slot:Arc<PublicationSlot>,
}
impl ControlsPublication {
    pub(crate) fn validate_apply(&self,params:&SessionControlsApplyParams)->SecurityResult<Option<usize>> {
        if self.catalogue.target!=params.target||self.catalogue.catalogue_generation!=params.catalogue_generation
            ||Instant::now()>=self.expires {return Err(ErrorCode::StaleSession)}
        let reference=params.model_ref.as_ref().or(self.catalogue.current.model_ref.as_ref());
        let index=reference.map(|reference|self.catalogue.models.iter().position(|model|&model.reference==reference)
            .ok_or(ErrorCode::InvalidRequest)).transpose()?;
        if index.is_some_and(|index|index>=self.models.len()) {return Err(ErrorCode::StaleSession)}
        if let Some(effort)=&params.effort {
            let model=index.and_then(|index|self.catalogue.models.get(index)).ok_or(ErrorCode::NotReady)?;
            if !model.efforts.contains(effort) {return Err(ErrorCode::InvalidRequest)}
        }
        if let Some(mode)=params.mode {
            if !self.catalogue.modes.contains(&mode) {return Err(ErrorCode::InvalidRequest)}
            if params.model_ref.is_none()&&self.current_native.is_none() {return Err(ErrorCode::NotReady)}
        }
        Ok(index)
    }
    pub(crate) fn desired_model(&self,index:Option<usize>)->Option<&NativeModelIdentity> {
        index.and_then(|index|self.models.get(index)).map(|model|&model.identity).or(self.current_native.as_ref())
    }
}
pub(crate) struct QueueRow {pub(crate) id:Id,pub(crate) text:String,pub(crate) editable:bool}
#[derive(Clone)]
pub(crate) struct QueueEntryBinding {
    pub(crate) id:Id,
    pub(crate) editable:bool,
}
pub(crate) struct QueuePublication {
    pub(crate) origin:PublicationOrigin,
    pub(crate) entries:Vec<QueueEntryBinding>,
    pub(crate) next_cursor:Option<Id>,
    pub(crate) native_cursor:Option<String>,
    pub(crate) complete:bool,
    pub(crate) bytes:usize,
    pub(crate) payloads:Vec<Uuid>,
    pub(crate) observation:Uuid,
    pub(crate) expires:Instant,
    pub(crate) slot:Arc<PublicationSlot>,
}
