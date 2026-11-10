// SPDX-License-Identifier: GPL-3.0-or-later
//! Real system-SQLite receipt workflow, not provider/authentication evidence.
use goatr::{protocol::*, storage::*};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("goatr-sqlite-smoke-{}", Uuid::random().0));
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    let uid = fs::metadata(&root)?.uid();
    let path = root.join("state.sqlite");
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Storage::open(&path, [42; 32], 0, uid)?;
        // Isolated fixture identity/key: no production pair, key provider, or native write.
        let device = PairedDevice {
            device_id: Uuid::random(),
            endpoint_id: EndpointId::new(
                iroh::SecretKey::from_bytes(&[43; 32]).public().to_string(),
            )?,
            device_name: Label::new("isolated SQLite smoke")?,
            paired_at: Uint(0),
            last_seen_at: None,
        };
        let epoch = store.pair_device(&device)?;
        let peer = store.authenticated_hello(&device.device_id, epoch)?;
        let request = Request {
            id: Uuid::random(),
            body: RequestBody::PromptSubmit(PromptSubmitParams {
                delivery: None,
                native_turn_id: None,
                images: None,
                skill: None,
                boot_id: store.boot_id.clone(),
                target: Target {
                    session_id: Uuid::random(),
                    generation: Uuid::random(),
                },
                text: PromptText::new("not stored in receipt database")?,
            }),
        };
        assert!(matches!(
            store.reserve(&peer, &request, 1, |_| Ok(()))?,
            Reservation::Reserved(_)
        ));
        store.transition(
            &peer,
            &request.id,
            ReceiptTransition {
                state: ReceiptState::Dispatching,
                proof: TransitionProof::Intent,
                result: None,
                error: None,
                at: 2,
            },
        )?;
        drop(store);
        let mut reopened = Storage::open(&path, [42; 32], 3, uid)?;
        assert_eq!(
            reopened
                .status(&peer, &request.id, 3)?
                .expect("receipt")
                .state,
            ReceiptState::Uncertain
        );
        assert!(matches!(
            reopened.reserve(&peer, &request, 4, |_| panic!("must not replay"))?,
            Reservation::Existing(_)
        ));
        reopened.revoke_epoch(peer.peer_id(), 4)?;
        assert!(matches!(
            reopened.status(&peer, &request.id, 4),
            Err(StorageError::Policy(ErrorCode::Unauthorized))
        ));
        println!(
            "system SQLite {}: durable pre-write crash -> uncertain; matching stale lookup -> no replay; revocation -> inaccessible",
            rusqlite::version()
        );
        Ok(())
    })();
    fs::remove_dir_all(&root)?;
    result
}
