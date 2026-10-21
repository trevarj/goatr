// SPDX-License-Identifier: GPL-3.0-or-later
use goatr::{
    codex_relay::{Observer, private_read, private_write},
    config,
    protocol::{Id, Uuid},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};
#[test]
fn typed_large_signed_ids_bind_only_exact_correlated_tui_success() {
    let mut observer = Observer::default();
    assert!(
        !observer
            .from_owner(br#"{"method":"thread/started","params":{"thread":{"id":"unsolicited"}}}"#)
            .unwrap()
    );
    assert!(observer.associated().is_empty());
    observer.from_tui(br#"{"id":-123456789012345678901234567890,"method":"thread/resume","params":{"threadId":"T"}}"#).unwrap();
    assert!(!observer.from_owner(br#"{"id":"-123456789012345678901234567890","result":{"thread":{"id":"wrong-type"}}}"#).unwrap());
    assert!(observer.associated().is_empty());
    assert!(
        observer
            .from_owner(br#"{"id":-123456789012345678901234567890,"result":{"thread":{"id":"T"}}}"#)
            .unwrap()
    );
    assert_eq!(observer.associated(), &[Id("T".into())]);
    assert_eq!(observer.resumed(), &[Id("T".into())]);
    observer
        .from_tui(br#"{"id":"fork","method":"thread/fork","params":{"threadId":"T"}}"#)
        .unwrap();
    assert!(
        observer
            .from_owner(br#"{"id":"fork","result":{"thread":{"id":"child"}}}"#)
            .unwrap()
    );
    assert_eq!(observer.resumed(), &[Id("T".into())]);
    assert_eq!(observer.associated(), &[Id("T".into()), Id("child".into())]);
}
#[test]
fn first_malformed_reply_consumes_callback_and_substitute_resume_fails_closed() {
    let mut observer = Observer::default();
    observer
        .from_tui(br#"{"id":-0,"method":"thread/resume","params":{"threadId":"T"}}"#)
        .unwrap();
    assert!(
        !observer
            .from_owner(br#"{"id":0,"result":{"thread":{"id":"T"}}}"#)
            .unwrap()
    );
    assert!(observer.from_owner(br#"{"id":-0,"result":null}"#).is_err());
    assert!(
        !observer
            .from_owner(br#"{"id":-0,"result":{"thread":{"id":"T"}}}"#)
            .unwrap()
    );
    assert!(observer.associated().is_empty());
    observer
        .from_tui(br#"{"id":9,"method":"thread/resume","params":{"threadId":"T"}}"#)
        .unwrap();
    assert!(
        observer
            .from_owner(br#"{"id":9,"result":{"thread":{"id":"substitute"}}}"#)
            .is_err()
    );
    assert!(observer.associated().is_empty());
}
#[test]
fn unsuccessful_unsubscribe_keeps_association_success_clears_only_that_thread() {
    let mut observer = Observer::default();
    for (id, thread) in [("a", "T"), ("b", "U")] {
        observer
            .from_tui(
                &serde_json::to_vec(
                    &json!({"id":id,"method":"thread/resume","params":{"threadId":thread}}),
                )
                .unwrap(),
            )
            .unwrap();
        observer
            .from_owner(
                &serde_json::to_vec(&json!({"id":id,"result":{"thread":{"id":thread}}})).unwrap(),
            )
            .unwrap();
    }
    observer
        .from_tui(br#"{"id":"u1","method":"thread/unsubscribe","params":{"threadId":"T"}}"#)
        .unwrap();
    assert!(
        !observer
            .from_owner(br#"{"id":"u1","error":{"code":-1,"message":"native rejection"}}"#)
            .unwrap()
    );
    assert_eq!(observer.associated().len(), 2);
    observer
        .from_tui(br#"{"id":"u2","method":"thread/unsubscribe","params":{"threadId":"T"}}"#)
        .unwrap();
    assert!(
        observer
            .from_owner(br#"{"id":"u2","result":{"status":"unsubscribed"}}"#)
            .unwrap()
    );
    assert_eq!(observer.associated(), &[Id("U".into())]);
    assert_eq!(observer.resumed(), &[Id("U".into())]);
}
#[test]
fn duplicate_or_overflow_correlations_and_duplicate_native_map_keys_are_rejected() {
    let mut observer = Observer::default();
    observer
        .from_tui(br#"{"id":"one","method":"thread/start","params":{}}"#)
        .unwrap();
    assert!(
        observer
            .from_tui(br#"{"id":"one","method":"thread/resume","params":{"threadId":"T"}}"#)
            .is_err()
    );
    for n in 1..64 {
        observer
            .from_tui(
                &serde_json::to_vec(&json!({"id":n,"method":"thread/start","params":{}})).unwrap(),
            )
            .unwrap();
    }
    assert!(
        observer
            .from_tui(br#"{"id":65,"method":"thread/start","params":{}}"#)
            .is_err()
    );
    assert!(
        Observer::default()
            .from_tui(br#"{"id":1,"method":"thread/start","params":{"same":1,"\u0073ame":2}}"#)
            .is_err()
    );
}
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("gc-{}", &Uuid::random().0[..8]));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Private {
    v: u32,
    binding: Uuid,
    value: String,
}
#[test]
fn atomic_private_records_never_adopt_overwrite_alias_or_accept_oversized_partial_data() {
    let root = Root::new();
    let path = root.0.join("binding");
    let a = Private {
        v: 1,
        binding: Uuid::random(),
        value: "complete record".into(),
    };
    private_write(&path, &a, false).unwrap();
    assert_eq!(private_read::<Private>(&path).unwrap(), a);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let b = Private {
        v: 1,
        binding: Uuid::random(),
        value: "foreign replacement".into(),
    };
    assert!(private_write(&path, &b, false).is_err());
    assert_eq!(private_read::<Private>(&path).unwrap(), a);
    let alias = root.0.join("alias");
    fs::hard_link(&path, &alias).unwrap();
    assert!(private_read::<Private>(&path).is_err());
    assert!(private_write(&path, &b, true).is_err());
    fs::remove_file(&alias).unwrap();
    let unsafe_path = root.0.join("symlink");
    std::os::unix::fs::symlink(&path, &unsafe_path).unwrap();
    assert!(private_read::<Private>(&unsafe_path).is_err());
    let huge = root.0.join("huge");
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&huge)
        .unwrap()
        .set_len(65_537)
        .unwrap();
    assert!(private_read::<Private>(&huge).is_err());
    let partial = root.0.join("partial");
    std::io::Write::write_all(
        &mut fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&partial)
            .unwrap(),
        b"{\"v\":1",
    )
    .unwrap();
    assert!(private_read::<Private>(&partial).is_err());
}
#[test]
fn socket_byte_limits_apply_independently_to_both_views() {
    assert!(config::socket_path(std::path::Path::new(&format!("/{}", "a".repeat(106)))).is_ok());
    assert!(config::socket_path(std::path::Path::new(&format!("/{}", "a".repeat(107)))).is_err());
    assert!(config::socket_path(std::path::Path::new(&format!("/{}", "é".repeat(54)))).is_err());
}
#[test]
fn arbitrary_process_or_claimed_epoch_cannot_pass_publisher_native_gate() {
    let actual = goatr::herdr::process_evidence(std::process::id()).unwrap();
    assert!(goatr::codex_relay::verify_distribution(&actual).is_err());
    let mut reused = actual;
    reused.start_ticks += 1;
    assert!(goatr::codex_relay::verify_distribution(&reused).is_err());
}

#[test]
fn exact_resume_rejection_is_not_association_and_internal_error_stays_unknown() {
    let mut observer = Observer::default();
    observer
        .from_tui(br#"{"id":"empty","method":"thread/resume","params":{"threadId":"T"}}"#)
        .unwrap();
    assert!(observer.from_owner(br#"{"id":"empty","error":{"code":-32600,"message":"no rollout found for thread id T"}}"#).unwrap());
    assert_eq!(observer.rejected_resume(), Some(&Id("T".into())));
    assert!(observer.associated().is_empty());
    assert!(observer.resumed().is_empty());
    assert!(
        !observer
            .from_owner(br#"{"id":"empty","result":{"thread":{"id":"T"}}}"#)
            .unwrap()
    );
    let mut unknown = Observer::default();
    unknown
        .from_tui(br#"{"id":"uncertain","method":"thread/resume","params":{"threadId":"T"}}"#)
        .unwrap();
    unknown.from_owner(br#"{"id":"uncertain","error":{"code":-32603,"message":"internal after possible attachment"}}"#).unwrap();
    assert!(unknown.rejected_resume().is_none());
    assert!(unknown.associated().is_empty());
    assert!(unknown.resumed().is_empty());
}
