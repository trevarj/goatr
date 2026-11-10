// SPDX-License-Identifier: GPL-3.0-or-later
//! SQLite transactions own endpoint rotation, probe deadlines and hint jobs.
use super::*;
use crate::push::{Attempt, Publication, Registration};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};

fn seal(
    key: &[u8; 32],
    peer: &Uuid,
    generation: &Uuid,
    value: &Registration,
) -> StoreResult<Vec<u8>> {
    let derived = hmac_sha256(key, b"goatr private push registration v1");
    let cipher = Aes256Gcm::new_from_slice(&derived).map_err(|_| StorageError::Corrupt)?;
    let random = crate::pairing::random32()?;
    let nonce = &random[..12];
    let aad = format!("push:{}:{}", peer.0, generation.0);
    let mut plain = serde_json::to_vec(value).map_err(|_| StorageError::Corrupt)?;
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: &plain,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| StorageError::Corrupt);
    plain.fill(0);
    let mut bytes = vec![1];
    bytes.extend_from_slice(nonce);
    bytes.extend_from_slice(&encrypted?);
    Ok(bytes)
}
fn unseal(
    key: &[u8; 32],
    peer: &Uuid,
    generation: &Uuid,
    bytes: &[u8],
) -> StoreResult<Registration> {
    if bytes.len() < 29 || bytes.len() > 16384 || bytes[0] != 1 {
        return Err(StorageError::Corrupt);
    }
    let derived = hmac_sha256(key, b"goatr private push registration v1");
    let cipher = Aes256Gcm::new_from_slice(&derived).map_err(|_| StorageError::Corrupt)?;
    let aad = format!("push:{}:{}", peer.0, generation.0);
    let mut plain = cipher
        .decrypt(
            Nonce::from_slice(&bytes[1..13]),
            Payload {
                msg: &bytes[13..],
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| StorageError::Corrupt)?;
    let result = serde_json::from_slice::<Registration>(&plain).map_err(|_| StorageError::Corrupt);
    plain.fill(0);
    let value = result?;
    if value.peer != *peer
        || value.generation != *generation
        || value.deadline
            != value
                .registered
                .checked_add(45000)
                .ok_or(StorageError::Corrupt)?
    {
        return Err(StorageError::Corrupt);
    }
    value.key.validate()?;
    value.probe.validate()?;
    Ok(value)
}
fn load(db: &Connection, key: &[u8; 32], peer: &Uuid) -> StoreResult<Option<Registration>> {
    let row: Option<(String, Vec<u8>)> = db
        .query_row(
            "SELECT generation,ciphertext FROM private_registration WHERE peer=?1",
            [&peer.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(generation, bytes)| unseal(key, peer, &Uuid::new(generation)?, &bytes))
        .transpose()
}
fn save(db: &Connection, key: &[u8; 32], value: &Registration) -> StoreResult<()> {
    db.execute("INSERT INTO private_registration(peer,generation,ciphertext) VALUES(?1,?2,?3) ON CONFLICT(peer) DO UPDATE SET generation=excluded.generation,ciphertext=excluded.ciphertext",params![value.peer.0,value.generation.0,seal(key,&value.peer,&value.generation,value)?])?;
    Ok(())
}
fn retire(tx: &Transaction<'_>, peer: &Uuid, generation: &Uuid, now: u64) -> StoreResult<()> {
    if now >= RETENTION_MS {
        tx.execute(
            "DELETE FROM retired_push WHERE peer=?1 AND at<=?2",
            params![peer.0, sql_integer(now - RETENTION_MS)?],
        )?;
    }
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM retired_push WHERE peer=?1",
        [&peer.0],
        |r| r.get(0),
    )?;
    if count >= 10000 {
        return reject(ErrorCode::ResourceLimit);
    }
    tx.execute(
        "INSERT OR IGNORE INTO retired_push(peer,generation,at) VALUES(?1,?2,?3)",
        params![peer.0, generation.0, sql_integer(now)?],
    )?;
    Ok(())
}
pub(super) fn enqueue_event(
    tx: &Transaction<'_>,
    peer: &str,
    event: &AttentionEvent,
    now: u64,
) -> StoreResult<()> {
    let generation: Option<String> = tx
        .query_row(
            "SELECT generation FROM private_registration WHERE peer=?1",
            [peer],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(generation) = generation {
        tx.execute("INSERT OR IGNORE INTO push_jobs(peer,id,generation,probe,attempts,due,expires) VALUES(?1,?2,?3,0,0,?4,?5)",params![peer,event.event_id.0,generation,sql_integer(now)?,sql_integer(now.checked_add(3600000).ok_or(StorageError::Corrupt)?)?])?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM push_jobs WHERE peer=?1 AND probe=0",
            [peer],
            |r| r.get(0),
        )?;
        if count > 100 {
            tx.execute("DELETE FROM push_jobs WHERE peer=?1 AND probe=0 AND id IN(SELECT id FROM push_jobs WHERE peer=?1 AND probe=0 ORDER BY due,id LIMIT ?2)",params![peer,count-100])?;
            tx.execute("INSERT INTO push_sync(peer,required) VALUES(?1,1) ON CONFLICT(peer) DO UPDATE SET required=1",[peer])?;
        }
    }
    Ok(())
}
impl Storage {
    pub(crate) fn push_register(
        &mut self,
        proof: &AuthenticatedPeer,
        p: PushRegisterParams,
        now: u64,
    ) -> StoreResult<(PushRegisterResult, bool)> {
        if p.boot_id != self.boot_id {
            return reject(ErrorCode::StaleHost);
        }
        p.validate()?;
        let key = self.receipt_key;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        let retired: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM retired_push WHERE peer=?1 AND generation=?2)",
            params![proof.peer.0, p.endpoint_generation.0],
            |r| r.get(0),
        )?;
        if retired {
            return reject(ErrorCode::PushRejected);
        }
        if let Some(old) = load(&tx, &key, &proof.peer)? {
            if old.generation == p.endpoint_generation {
                if old.epoch != proof.epoch
                    || old.endpoint != p.endpoint
                    || old.key != p.push_key
                    || old.probe != p.probe
                {
                    return reject(ErrorCode::PushRejected);
                }
                let result = PushRegisterResult {
                    registered: TrueValue(true),
                    endpoint_generation: old.generation,
                    probe: old.probe,
                    verified: old.verified && !old.disabled,
                };
                tx.commit()?;
                return Ok((result, false));
            }
            retire(&tx, &proof.peer, &old.generation, now)?;
        }
        tx.execute("DELETE FROM push_jobs WHERE peer=?1", [&proof.peer.0])?;
        let value = Registration {
            peer: proof.peer.clone(),
            epoch: proof.epoch,
            generation: p.endpoint_generation,
            endpoint: p.endpoint,
            key: p.push_key,
            probe: p.probe,
            registered: now,
            deadline: now.checked_add(45000).ok_or(StorageError::Corrupt)?,
            verified: false,
            disabled: false,
        };
        save(&tx, &key, &value)?;
        tx.execute("INSERT INTO push_jobs(peer,id,generation,probe,attempts,due,expires) VALUES(?1,?2,?3,1,0,?4,?5)",params![proof.peer.0,value.probe.0,value.generation.0,sql_integer(now)?,sql_integer(value.deadline)?])?;
        tx.commit()?;
        Ok((
            PushRegisterResult {
                registered: TrueValue(true),
                endpoint_generation: value.generation,
                probe: value.probe,
                verified: false,
            },
            true,
        ))
    }
    pub(crate) fn push_ack(
        &mut self,
        proof: &AuthenticatedPeer,
        p: PushProbeAckParams,
        now: u64,
    ) -> StoreResult<PushProbeAckResult> {
        if p.boot_id != self.boot_id {
            return reject(ErrorCode::StaleHost);
        }
        let key = self.receipt_key;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        let mut value = load(&tx, &key, &proof.peer)?
            .ok_or(StorageError::Policy(ErrorCode::StaleRegistration))?;
        if value.epoch != proof.epoch
            || value.generation != p.endpoint_generation
            || value.probe != p.probe
            || value.disabled
            || !value.verified && now >= value.deadline
        {
            return reject(ErrorCode::StaleRegistration);
        }
        value.verified = true;
        save(&tx, &key, &value)?;
        tx.execute(
            "DELETE FROM push_jobs WHERE peer=?1 AND probe=1",
            [&proof.peer.0],
        )?;
        tx.commit()?;
        Ok(PushProbeAckResult {
            verified: TrueValue(true),
        })
    }
    pub(crate) fn push_unregister(
        &mut self,
        proof: &AuthenticatedPeer,
        p: PushUnregisterParams,
        now: u64,
    ) -> StoreResult<PushUnregisterResult> {
        if p.boot_id != self.boot_id {
            return reject(ErrorCode::StaleHost);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        let current: Option<String> = tx
            .query_row(
                "SELECT generation FROM private_registration WHERE peer=?1",
                [&proof.peer.0],
                |r| r.get(0),
            )
            .optional()?;
        let matches = current.as_deref() == Some(p.endpoint_generation.0.as_str());
        if matches {
            retire(&tx, &proof.peer, &p.endpoint_generation, now)?;
            tx.execute(
                "DELETE FROM private_registration WHERE peer=?1",
                [&proof.peer.0],
            )?;
            tx.execute("DELETE FROM push_jobs WHERE peer=?1", [&proof.peer.0])?;
        }
        tx.commit()?;
        Ok(PushUnregisterResult {
            unregistered: matches,
        })
    }
    pub(crate) fn push_next(
        &mut self,
        now: u64,
        excluded: &[Uuid],
    ) -> StoreResult<Option<Attempt>> {
        let key = self.receipt_key;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM push_jobs WHERE expires<=?1",
            [sql_integer(now)?],
        )?;
        let rows: Vec<(String, String, bool, i64, i64, i64)> = {
            let mut s=tx.prepare("SELECT j.peer,j.id,j.probe,j.attempts,j.expires,p.epoch FROM push_jobs j JOIN peers p USING(peer) JOIN private_registration r ON r.peer=j.peer AND r.generation=j.generation WHERE due<=?1 AND p.paired=1 AND NOT EXISTS(SELECT 1 FROM json_each(?2) excluded WHERE excluded.value=j.peer) ORDER BY due,j.peer,j.id LIMIT 100")?;
            s.query_map(params![sql_integer(now)?, json(&excluded)?], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<Result<_, _>>()?
        };
        let mut chosen = None;
        for (peer, id, probe, attempts, expires, epoch) in rows {
            let peer = Uuid::new(peer)?;
            let id = Uuid::new(id)?;
            let Some(registration) = load(&tx, &key, &peer)? else {
                continue;
            };
            if registration.epoch != stored_u64(epoch)?
                || registration.disabled
                || probe && (registration.verified || now >= registration.deadline)
            {
                tx.execute(
                    "DELETE FROM push_jobs WHERE peer=?1 AND id=?2",
                    params![peer.0, id.0],
                )?;
                continue;
            }
            if !probe && !registration.verified {
                if now >= registration.deadline {
                    tx.execute("DELETE FROM push_jobs WHERE peer=?1", [&peer.0])?;
                } else {
                    tx.execute(
                        "UPDATE push_jobs SET due=?2 WHERE peer=?1 AND probe=0",
                        params![
                            peer.0,
                            sql_integer(now.checked_add(5000).ok_or(StorageError::Corrupt)?)?
                        ],
                    )?;
                }
                continue;
            }
            let event = if probe {
                None
            } else {
                let value:Option<String>=tx.query_row("SELECT event FROM attention WHERE peer=?1 AND json_extract(event,'$.eventId')=?2",params![peer.0,id.0],|r|r.get(0)).optional()?;
                let Some(value) = value else {
                    tx.execute(
                        "DELETE FROM push_jobs WHERE peer=?1 AND id=?2",
                        params![peer.0, id.0],
                    )?;
                    continue;
                };
                let event: AttentionEvent = decode(&value)?;
                let through: Option<i64> = tx
                    .query_row(
                        "SELECT through FROM readwater WHERE peer=?1 AND session=?2",
                        params![peer.0, event.session_id.0],
                        |r| r.get(0),
                    )
                    .optional()?;
                if through.is_some_and(|n| u64::try_from(n).is_ok_and(|n| n >= event.sequence.0)) {
                    tx.execute(
                        "DELETE FROM push_jobs WHERE peer=?1 AND id=?2",
                        params![peer.0, id.0],
                    )?;
                    continue;
                }
                Some(event)
            };
            tx.execute(
                "UPDATE push_jobs SET due=?3 WHERE peer=?1 AND id=?2",
                params![
                    peer.0,
                    id.0,
                    sql_integer(now.checked_add(10000).ok_or(StorageError::Corrupt)?)?
                ],
            )?;
            chosen = Some(Attempt {
                proof: AuthenticatedPeer {
                    peer: peer.clone(),
                    epoch: stored_u64(epoch)?,
                },
                registration,
                id,
                event,
                attempts: stored_u64(attempts)?,
                expires: stored_u64(expires)?,
            });
            break;
        }
        tx.commit()?;
        Ok(chosen)
    }
    pub(crate) fn push_finish(
        &mut self,
        attempt: Attempt,
        result: Publication,
        now: u64,
    ) -> StoreResult<()> {
        let key = self.receipt_key;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if authorize(&tx, &attempt.proof).is_err() {
            tx.commit()?;
            return Ok(());
        }
        let Some(mut value) = load(&tx, &key, attempt.proof.peer_id())? else {
            tx.commit()?;
            return Ok(());
        };
        if value.generation != attempt.registration.generation {
            tx.commit()?;
            return Ok(());
        }
        let peer = &value.peer;
        let probe = attempt.event.is_none();
        let remove = matches!(result, Publication::Disabled | Publication::Rejected)
            || !probe && matches!(result, Publication::Accepted)
            || now >= attempt.expires;
        if matches!(result, Publication::Disabled | Publication::Rejected) {
            value.disabled = true;
            save(&tx, &key, &value)?;
            tx.execute("DELETE FROM push_jobs WHERE peer=?1", [&peer.0])?;
        } else if remove {
            tx.execute(
                "DELETE FROM push_jobs WHERE peer=?1 AND id=?2",
                params![peer.0, attempt.id.0],
            )?;
        } else {
            let retry = [5u64, 15, 30, 60, 120, 300];
            let seconds = if probe {
                5
            } else {
                retry[usize::try_from(attempt.attempts)
                    .unwrap_or(usize::MAX)
                    .min(retry.len() - 1)]
            };
            let seconds = if !probe && let Publication::Retry { after: Some(after) } = result {
                seconds.max(after)
            } else {
                seconds
            };
            let due = now
                .checked_add(
                    seconds
                        .min(3600)
                        .checked_mul(1000)
                        .ok_or(StorageError::Corrupt)?,
                )
                .ok_or(StorageError::Corrupt)?
                .min(attempt.expires);
            tx.execute(
                "UPDATE push_jobs SET due=?3,attempts=?4 WHERE peer=?1 AND id=?2 AND generation=?5",
                params![
                    peer.0,
                    attempt.id.0,
                    sql_integer(due)?,
                    sql_integer(
                        attempt
                            .attempts
                            .checked_add(1)
                            .ok_or(StorageError::Corrupt)?
                    )?,
                    value.generation.0
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
impl Storage {
    pub(crate) fn push_attempt_current(&self, attempt: &Attempt, now: u64) -> StoreResult<bool> {
        authorize(&self.db, &attempt.proof)?;
        let Some(value) = load(&self.db, &self.receipt_key, attempt.proof.peer_id())? else {
            return Ok(false);
        };
        if value.generation != attempt.registration.generation
            || value.epoch != attempt.proof.epoch
            || value.disabled
            || value.endpoint != attempt.registration.endpoint
            || value.key != attempt.registration.key
            || now >= attempt.expires
        {
            return Ok(false);
        }
        if let Some(event) = &attempt.event {
            if !value.verified {
                return Ok(false);
            }
            let unread:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM attention a WHERE a.peer=?1 AND a.sequence=?2 AND NOT EXISTS(SELECT 1 FROM readwater r WHERE r.peer=a.peer AND r.session=a.session AND r.through>=a.sequence))",params![value.peer.0,sql_integer(event.sequence.0)?],|r|r.get(0))?;
            Ok(unread)
        } else {
            Ok(!value.verified && value.probe == attempt.id && now < value.deadline)
        }
    }
    pub(crate) fn attention_peer(&self, peer: &Uuid) -> StoreResult<Option<AuthenticatedPeer>> {
        let epoch: Option<i64> = self
            .db
            .query_row(
                "SELECT epoch FROM peers WHERE peer=?1 AND paired=1",
                [&peer.0],
                |r| r.get(0),
            )
            .optional()?;
        epoch
            .map(|epoch| {
                Ok(AuthenticatedPeer {
                    peer: peer.clone(),
                    epoch: stored_u64(epoch)?,
                })
            })
            .transpose()
    }
    pub(crate) fn push_sync_needed(&self, proof: &AuthenticatedPeer) -> StoreResult<bool> {
        authorize(&self.db, proof)?;
        Ok(self
            .db
            .query_row(
                "SELECT required FROM push_sync WHERE peer=?1",
                [&proof.peer.0],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }
}

impl Storage {
    pub(crate) fn omp_cycle_install(
        &mut self,
        target: &Target,
        source: &Uuid,
        connection: &Uuid,
        status: SessionStatus,
        now: u64,
    ) -> StoreResult<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM attention_omp_cycles WHERE active IS NULL AND NOT EXISTS(SELECT 1 FROM sessions WHERE sessions.session=attention_omp_cycles.session)",[])?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM attention_omp_cycles WHERE session<>?1",
            [&target.session_id.0],
            |r| r.get(0),
        )?;
        if count >= 512 {
            return reject(ErrorCode::ResourceLimit);
        }
        tx.execute("INSERT INTO attention_omp_cycles(session,generation,source,connection,ordinal,active) VALUES(?1,?2,?3,?4,0,NULL) ON CONFLICT(session) DO UPDATE SET generation=excluded.generation,source=excluded.source,connection=excluded.connection,active=NULL",params![target.session_id.0,target.generation.0,source.0,connection.0])?;
        attention_transition_tx(
            &tx,
            &AttentionTransition {
                target: target.clone(),
                correlation: Id("snapshot".into()),
                previous: status,
                current: status,
                snapshot: true,
                new_actionable: false,
            },
            now,
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn omp_cycle_retire(
        &mut self,
        target: &Target,
        source: &Uuid,
        connection: &Uuid,
    ) -> StoreResult<()> {
        self.db.execute("UPDATE attention_omp_cycles SET active=NULL WHERE session=?1 AND generation=?2 AND source=?3 AND connection=?4",params![target.session_id.0,target.generation.0,source.0,connection.0])?;
        Ok(())
    }
    pub(crate) fn omp_cycle(
        &mut self,
        target: &Target,
        source: &Uuid,
        connection: &Uuid,
        start: bool,
        now: u64,
    ) -> StoreResult<Vec<(Uuid, AttentionEvent)>> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row:Option<(i64,Option<String>)>=tx.query_row("SELECT ordinal,active FROM attention_omp_cycles WHERE session=?1 AND generation=?2 AND source=?3 AND connection=?4",params![target.session_id.0,target.generation.0,source.0,connection.0],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((ordinal, active)) = row else {
            tx.commit()?;
            return Ok(Vec::new());
        };
        if ordinal < 0 {
            return Err(StorageError::Corrupt);
        }
        let mut events = Vec::new();
        if start && active.is_none() {
            let ordinal = ordinal.checked_add(1).ok_or(StorageError::Corrupt)?;
            let token = Uuid::random();
            tx.execute(
                "UPDATE attention_omp_cycles SET ordinal=?2,active=?3 WHERE session=?1",
                params![target.session_id.0, ordinal, token.0],
            )?;
            attention_transition_tx(
                &tx,
                &AttentionTransition {
                    target: target.clone(),
                    correlation: Id(format!("omp-cycle-start:{}", token.0)),
                    previous: SessionStatus::Unknown,
                    current: SessionStatus::Working,
                    snapshot: true,
                    new_actionable: false,
                },
                now,
            )?;
        } else if !start && let Some(active) = active {
            tx.execute(
                "UPDATE attention_omp_cycles SET active=NULL WHERE session=?1",
                [&target.session_id.0],
            )?;
            // Completion is this observed continuous source cycle, not a native turn ID.
            // Intermediate request/turn/status checkpoints cannot erase its start proof.
            tx.execute("INSERT INTO checkpoint(session,generation,status) VALUES(?1,?2,?3) ON CONFLICT(session) DO UPDATE SET generation=excluded.generation,status=excluded.status",params![target.session_id.0,target.generation.0,json(&SessionStatus::Working)?])?;
            events = attention_transition_tx(
                &tx,
                &AttentionTransition {
                    target: target.clone(),
                    correlation: Id(format!("omp-observed-cycle:{active}")),
                    previous: SessionStatus::Working,
                    current: SessionStatus::Idle,
                    snapshot: false,
                    new_actionable: false,
                },
                now,
            )?;
        }
        tx.commit()?;
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    fn paired(store: &mut Storage) -> AuthenticatedPeer {
        let device = PairedDevice {
            device_id: Uuid::random(),
            endpoint_id: EndpointId::new(
                iroh::SecretKey::from_bytes(&[71; 32]).public().to_string(),
            )
            .unwrap(),
            device_name: Label::new("owned push fixture").unwrap(),
            paired_at: Uint(0),
            last_seen_at: None,
        };
        let epoch = store.pair_device(&device).unwrap();
        store.authenticated_hello(&device.device_id, epoch).unwrap()
    }

    fn registration(store: &Storage) -> PushRegisterParams {
        PushRegisterParams {
            boot_id: store.boot_id.clone(),
            endpoint: "https://ntfy.sh/owned-public-fixture".into(),
            endpoint_generation: Uuid::random(),
            push_key: Key32(URL_SAFE_NO_PAD.encode([8; 32])),
            probe: Uuid::random(),
        }
    }

    fn ack(store: &Storage, value: &PushRegisterParams) -> PushProbeAckParams {
        PushProbeAckParams {
            boot_id: store.boot_id.clone(),
            endpoint_generation: value.endpoint_generation.clone(),
            probe: value.probe.clone(),
        }
    }

    #[test]
    fn idempotent_register_and_lost_ack_keep_original_deadline_but_disabled_verified_ack_refuses() {
        let mut store = Storage::memory([7; 32], 0).unwrap();
        let peer = paired(&mut store);
        let params = registration(&store);
        let (first, changed) = store.push_register(&peer, params.clone(), 100).unwrap();
        assert!(changed);
        assert!(!first.verified);
        let attempt = store.push_next(100, &[]).unwrap().unwrap();
        assert!(attempt.event.is_none());
        assert_eq!(attempt.id, params.probe);
        store
            .push_finish(attempt, Publication::Accepted, 101)
            .unwrap();
        let (retry, changed) = store.push_register(&peer, params.clone(), 40000).unwrap();
        assert!(!changed);
        assert!(!retry.verified);
        assert_eq!(
            load(&store.db, &store.receipt_key, peer.peer_id())
                .unwrap()
                .unwrap()
                .deadline,
            45100
        );
        store.push_ack(&peer, ack(&store, &params), 45099).unwrap();
        // A lost successful ACK reply may retry after the original unverified deadline.
        store.push_ack(&peer, ack(&store, &params), 90000).unwrap();
        assert!(
            store
                .push_register(&peer, params.clone(), 90001)
                .unwrap()
                .0
                .verified
        );
        let target = Target {
            session_id: Uuid::random(),
            generation: Uuid::random(),
        };
        let working = AttentionTransition {
            target,
            correlation: Id("native-working".into()),
            previous: SessionStatus::Idle,
            current: SessionStatus::Working,
            snapshot: true,
            new_actionable: false,
        };
        store.attention_transition(&working, 90001).unwrap();
        let completed = AttentionTransition {
            correlation: Id("native-completed".into()),
            previous: SessionStatus::Working,
            current: SessionStatus::Idle,
            snapshot: false,
            ..working
        };
        assert_eq!(
            store.attention_transition(&completed, 90001).unwrap().len(),
            1
        );
        let attempt = store.push_next(90002, &[]).unwrap().unwrap();
        assert!(attempt.event.is_some());
        // This is the real queued publisher result boundary, not a fabricated register result.
        store
            .push_finish(attempt, Publication::Disabled, 90002)
            .unwrap();
        assert!(
            !store
                .push_register(&peer, params.clone(), 90003)
                .unwrap()
                .0
                .verified
        );
        assert!(matches!(
            store.push_ack(&peer, ack(&store, &params), 90004),
            Err(StorageError::Policy(ErrorCode::StaleRegistration))
        ));
        let replacement = registration(&store);
        store
            .push_register(&peer, replacement.clone(), 90005)
            .unwrap();
        assert!(matches!(
            store.push_register(&peer, params, 90006),
            Err(StorageError::Policy(ErrorCode::PushRejected))
        ));
        store.revoke_epoch(peer.peer_id(), 90007).unwrap();
        assert!(matches!(
            store.push_ack(&peer, ack(&store, &replacement), 90008),
            Err(StorageError::Policy(ErrorCode::Unauthorized))
        ));
    }

    #[test]
    fn observed_omp_cycle_deduplicates_real_start_end_and_excludes_snapshot_retirement_and_foreign_source()
     {
        let mut store = Storage::memory([7; 32], 0).unwrap();
        let peer = paired(&mut store);
        let target = Target {
            session_id: Uuid::random(),
            generation: Uuid::random(),
        };
        let source = Uuid::random();
        let connection = Uuid::random();
        store
            .omp_cycle_install(&target, &source, &connection, SessionStatus::Working, 0)
            .unwrap();
        assert!(
            store
                .omp_cycle(&target, &source, &connection, false, 1)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .omp_cycle(&target, &Uuid::random(), &connection, true, 2)
                .unwrap()
                .is_empty()
        );
        store
            .omp_cycle(&target, &source, &connection, true, 3)
            .unwrap();
        store
            .omp_cycle(&target, &source, &connection, true, 4)
            .unwrap();
        let checkpoint = AttentionTransition {
            target: target.clone(),
            correlation: Id("registry-checkpoint".into()),
            previous: SessionStatus::Working,
            current: SessionStatus::Idle,
            snapshot: true,
            new_actionable: false,
        };
        assert!(
            store
                .attention_transition(&checkpoint, 5)
                .unwrap()
                .is_empty()
        );
        let events = store
            .omp_cycle(&target, &source, &connection, false, 6)
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(&events[0].0, peer.peer_id());
        assert_eq!(events[0].1.category, AttentionCategory::Completed);
        assert!(
            store
                .omp_cycle(&target, &source, &connection, false, 7)
                .unwrap()
                .is_empty()
        );
        store
            .omp_cycle(&target, &source, &connection, true, 8)
            .unwrap();
        store
            .omp_cycle_retire(&target, &source, &connection)
            .unwrap();
        assert!(
            store
                .omp_cycle(&target, &source, &connection, false, 9)
                .unwrap()
                .is_empty()
        );
        store
            .omp_cycle_install(
                &target,
                &source,
                &Uuid::random(),
                SessionStatus::Working,
                10,
            )
            .unwrap();
        assert!(
            store
                .omp_cycle(&target, &source, &connection, false, 11)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn reopening_keeps_unverified_deadline_verified_ack_and_monotonic_cycle_retirement() {
        use std::os::unix::fs::DirBuilderExt;
        struct Root(std::path::PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let root = Root(
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(format!(".goatr-push-state-{}", Uuid::random().0)),
        );
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root.0)
            .unwrap();
        let path = root.0.join("state.sqlite");
        let mut store = Storage::open(&path, [7; 32], 0, crate::pairing::uid()).unwrap();
        let proof = paired(&mut store);
        let mut params = registration(&store);
        store.push_register(&proof, params.clone(), 100).unwrap();
        let target = Target {
            session_id: Uuid::random(),
            generation: Uuid::random(),
        };
        let source = Uuid::random();
        let connection = Uuid::random();
        store
            .omp_cycle_install(&target, &source, &connection, SessionStatus::Idle, 101)
            .unwrap();
        store
            .omp_cycle(&target, &source, &connection, true, 102)
            .unwrap();
        drop(store);
        let mut store = Storage::open(&path, [7; 32], 200, crate::pairing::uid()).unwrap();
        let proof = store
            .authenticated_hello(proof.peer_id(), proof.epoch())
            .unwrap();
        params.boot_id = store.boot_id.clone();
        assert!(
            !store
                .push_register(&proof, params.clone(), 40000)
                .unwrap()
                .0
                .verified
        );
        assert_eq!(
            load(&store.db, &store.receipt_key, proof.peer_id())
                .unwrap()
                .unwrap()
                .deadline,
            45100
        );
        assert!(
            store
                .omp_cycle(&target, &source, &connection, false, 40001)
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            store.push_ack(&proof, ack(&store, &params), 45100),
            Err(StorageError::Policy(ErrorCode::StaleRegistration))
        ));
        let mut next = registration(&store);
        store.push_register(&proof, next.clone(), 45101).unwrap();
        store.push_ack(&proof, ack(&store, &next), 45102).unwrap();
        drop(store);
        let mut store = Storage::open(&path, [7; 32], 100000, crate::pairing::uid()).unwrap();
        let proof = store
            .authenticated_hello(proof.peer_id(), proof.epoch())
            .unwrap();
        next.boot_id = store.boot_id.clone();
        assert!(
            store
                .push_register(&proof, next.clone(), 100001)
                .unwrap()
                .0
                .verified
        );
        store.push_ack(&proof, ack(&store, &next), 100002).unwrap();
    }
}
