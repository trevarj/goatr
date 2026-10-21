// SPDX-License-Identifier: GPL-3.0-or-later
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use goatr::{
    admin,
    pairing::*,
    protocol::*,
    state::State,
    storage::{Storage, StorageOwner},
    transport,
};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    sync::Arc,
    time::Duration,
};
fn new_security() -> Arc<Security> {
    let storage = Storage::memory([9; 32], now_ms()).unwrap();
    let boot = storage.boot_id.clone();
    Security::new(
        StorageOwner::new(storage).unwrap(),
        Arc::new(State::default()),
        EndpointId(iroh::SecretKey::from_bytes(&[3; 32]).public().to_string()),
        boot,
        vec![],
    )
}
fn new_root() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("goatr-pair-{}", Uuid::random().0));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    root
}
fn invite_uri(value: serde_json::Value) -> String {
    format!(
        "goatr://pair/v1?data={}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
    )
}
async fn direct(key: u8) -> iroh::Endpoint {
    goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[key; 32]))
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap()
}
async fn completed(host: &iroh::Endpoint, phone: &iroh::Endpoint) -> iroh::endpoint::Connection {
    let endpoint = host.clone();
    let accepting = tokio::spawn(async move { endpoint.accept().await.unwrap().await.unwrap() });
    let _client = phone.connect(host.addr(), goatr::ALPN).await.unwrap();
    accepting.await.unwrap()
}
#[tokio::test]
async fn exact_invite_atomic_consumption_restart_and_fail_limits() {
    let security = new_security();
    let pair = security.open_pair().await.unwrap();
    let invite = PairInvite::parse(&pair.uri).unwrap();
    assert!(PairInvite::parse(&(pair.uri.clone() + "&x=1")).is_err());
    assert!(PairInvite::parse(&(pair.uri.clone() + "=")).is_err());
    let mut value = serde_json::to_value(&invite).unwrap();
    value["unknown"] = serde_json::json!(1);
    assert!(PairInvite::parse(&invite_uri(value)).is_err());
    let host = direct(3).await;
    let phone_a = direct(4).await;
    let phone_b = direct(5).await;
    let connection_a = completed(&host, &phone_a).await;
    let connection_b = completed(&host, &phone_b).await;
    let first = EndpointId(connection_a.remote_id().to_string());
    let a = PairConfirmParams {
        token: invite.token.clone(),
        device_name: Label::new("phone").unwrap(),
    };
    let b = a.clone();
    let (left, right) = tokio::join!(
        security.confirm(&connection_a, a),
        security.confirm(&connection_b, b)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    assert_eq!(security.devices().await.unwrap().len(), 1);
    let retry = security
        .confirm(
            &connection_a,
            PairConfirmParams {
                token: invite.token,
                device_name: Label::new("phone").unwrap(),
            },
        )
        .await;
    assert_eq!(retry.unwrap_err(), ErrorCode::PairingFailed);
    for _ in 0..5 {
        drop(security.handshake(&first).unwrap());
    }
    assert!(security.handshake(&first).is_none());
    let permits = (0..8)
        .map(|_| security.admission().unwrap())
        .collect::<Vec<_>>();
    assert!(matches!(security.admission(), Err(ErrorCode::Busy)));
    drop(permits);
    let fresh = new_security();
    assert_eq!(
        fresh
            .confirm(
                &connection_a,
                PairConfirmParams {
                    token: Key32(URL_SAFE_NO_PAD.encode([0; 32])),
                    device_name: Label::new("phone").unwrap()
                }
            )
            .await
            .unwrap_err(),
        ErrorCode::PairingFailed
    );
    goatr::close_endpoint(&host).await.unwrap();
    goatr::close_endpoint(&phone_a).await.unwrap();
    goatr::close_endpoint(&phone_b).await.unwrap();
}
#[tokio::test]
async fn exact_admin_real_socket_and_cli_absence_never_owns_state() {
    let root = new_root();
    let path = root.join("admin.sock");
    let socket = admin::AdminSocket::bind(path.clone()).unwrap();
    let (stop, rx) = tokio::sync::watch::channel(false);
    let security = new_security();
    let job = tokio::spawn(async move { socket.serve(security, rx).await.unwrap() });
    assert_eq!(
        admin::call(&path, "devices.list", serde_json::json!({}))
            .await
            .unwrap(),
        serde_json::json!({"devices":[]})
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(
        admin::call(&path, "devices.list", serde_json::json!({}))
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = root.join("alias.sock");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(admin::validate_socket_path(&alias, false, true).is_err());
    fs::remove_file(alias).unwrap();
    assert!(
        admin::validate_socket_path(
            std::path::Path::new(&format!("/{}", "x".repeat(107))),
            false,
            false
        )
        .is_err()
    );
    assert!(admin::decode_request(br#"{"v":1,"id":"d9428888-122b-4e88-bb65-abe9954d9285","method":"pair.open","params":{},"params":{}}"#).is_err());
    assert!(
        admin::validate_socket_path(std::path::Path::new("relative.sock"), false, false).is_err()
    );
    let missing = root.join("missing.sock");
    let absent = std::process::Command::new(env!("CARGO_BIN_EXE_goatr"))
        .args(["devices", "--admin-socket"])
        .arg(&missing)
        .env("XDG_STATE_HOME", root.join("must-not-exist"))
        .output()
        .unwrap();
    assert!(!absent.status.success());
    assert!(!root.join("must-not-exist").exists());
    assert!(
        String::from_utf8(absent.stderr)
            .unwrap()
            .contains(missing.to_str().unwrap())
    );
    let nonterminal = std::process::Command::new(env!("CARGO_BIN_EXE_goatr"))
        .args(["pair", "--admin-socket"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!nonterminal.status.success());
    assert!(
        !String::from_utf8(nonterminal.stdout)
            .unwrap()
            .contains("goatr://")
    );
    let duplicate = std::process::Command::new(env!("CARGO_BIN_EXE_goatr"))
        .args(["serve", "--admin-socket"])
        .arg(&path)
        .env("XDG_STATE_HOME", root.join("must-not-exist"))
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    assert!(!root.join("must-not-exist").exists());
    let revoke = std::process::Command::new(env!("CARGO_BIN_EXE_goatr"))
        .arg("revoke")
        .arg(Uuid::random().0)
        .arg("--admin-socket")
        .arg(&missing)
        .env("XDG_STATE_HOME", root.join("must-not-exist"))
        .output()
        .unwrap();
    assert!(!revoke.status.success());
    assert!(!root.join("must-not-exist").exists());
    assert!(
        String::from_utf8(revoke.stderr)
            .unwrap()
            .contains(missing.to_str().unwrap())
    );
    stop.send(true).unwrap();
    job.await.unwrap();
    assert!(!path.exists());
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(admin::validate_socket_path(&missing, false, false).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn retained_identity_exclusive_corruption_and_loss_fail_closed() {
    let root = new_root();
    let owner = HostIdentity::acquire(root.clone()).unwrap();
    let id = owner.secret.public();
    assert!(HostIdentity::acquire(root.clone()).is_err());
    drop(owner);
    let retained = HostIdentity::acquire(root.clone()).unwrap();
    assert_eq!(retained.secret.public(), id);
    drop(retained);
    fs::write(root.join("state.sqlite3"), b"existing durable state").unwrap();
    fs::remove_file(root.join("identity.keys")).unwrap();
    assert!(HostIdentity::acquire(root.clone()).is_err());
    assert!(!root.join("identity.keys").exists());
    fs::remove_dir_all(root).unwrap();
    let root = new_root();
    let owner = HostIdentity::acquire(root.clone()).unwrap();
    drop(owner);
    fs::write(root.join("identity.keys"), b"corrupt").unwrap();
    assert!(HostIdentity::acquire(root.clone()).is_err());
    fs::remove_dir_all(root).unwrap();
    let root = new_root();
    let owner = HostIdentity::acquire(root.clone()).unwrap();
    drop(owner);
    fs::remove_file(root.join("identity.keys")).unwrap();
    assert!(HostIdentity::acquire(root.clone()).is_err());
    assert!(!root.join("identity.keys").exists());
    fs::remove_dir_all(root).unwrap();
}
async fn rpc(
    send: &mut iroh::endpoint::SendStream,
    recv: &mut iroh::endpoint::RecvStream,
    body: RequestBody,
) -> Response {
    let method = body.method();
    let id = Uuid::random();
    transport::write_frame(
        send,
        &encode_payload(&Envelope::Request(Request {
            id: id.clone(),
            body,
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    let bytes = tokio::time::timeout(
        Duration::from_secs(5),
        transport::read_frame(recv, FRAME_MAX),
    )
    .await
    .unwrap()
    .unwrap();
    let Envelope::Response(response) = decode_payload(&bytes, Some(method)).unwrap() else {
        panic!("expected response")
    };
    assert_eq!(response.id, id);
    response
}
#[tokio::test]
async fn real_direct_pair_reply_loss_known_hello_snapshot_unpair_and_no_pre_auth_reads() {
    let host = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[3; 32]))
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let phone = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[11; 32]))
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let security = new_security();
    let (stop, rx) = tokio::sync::watch::channel(false);
    let endpoint = host.clone();
    let s = security.clone();
    let server = tokio::spawn(async move {
        transport::serve(&endpoint, s, None, None, None, rx)
            .await
            .unwrap()
    });
    let unauthorized = phone.connect(host.addr(), goatr::ALPN).await.unwrap();
    let (mut send, mut recv) = unauthorized.open_bi().await.unwrap();
    let reply = rpc(
        &mut send,
        &mut recv,
        RequestBody::HostSnapshot(HostSnapshotParams {}),
    )
    .await;
    assert_eq!(reply.outcome.unwrap_err().code, ErrorCode::Unauthorized);
    unauthorized.close(0u32.into(), b"done");
    let invitation = PairInvite::parse(&security.open_pair().await.unwrap().uri).unwrap();
    let paired = phone.connect(host.addr(), goatr::ALPN).await.unwrap();
    let (mut send, _recv) = paired.open_bi().await.unwrap();
    let req = Request {
        id: Uuid::random(),
        body: RequestBody::PairConfirm(PairConfirmParams {
            token: invitation.token,
            device_name: Label::new("phone").unwrap(),
        }),
    };
    transport::write_frame(&mut send, &encode_payload(&Envelope::Request(req)).unwrap())
        .await
        .unwrap();
    // Observe the committed desktop record, intentionally never consume the pairing response.
    tokio::time::timeout(Duration::from_secs(5), async {
        while security.devices().await.unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    paired.close(0u32.into(), b"lost reply");
    let recovered = phone.connect(host.addr(), goatr::ALPN).await.unwrap();
    let (mut send, mut recv) = recovered.open_bi().await.unwrap();
    let hello = rpc(
        &mut send,
        &mut recv,
        RequestBody::Hello(HelloParams {
            app_version: "test".into(),
            device_name: Label::new("phone").unwrap(),
        }),
    )
    .await;
    let ResultBody::Hello(hello) = hello.outcome.unwrap() else {
        panic!("hello")
    };
    assert_eq!(
        hello.device_id,
        security.devices().await.unwrap()[0].device_id
    );
    let newest = phone.connect(host.addr(), goatr::ALPN).await.unwrap();
    let (mut send, mut recv) = newest.open_bi().await.unwrap();
    let replacement = rpc(
        &mut send,
        &mut recv,
        RequestBody::Hello(HelloParams {
            app_version: "test".into(),
            device_name: Label::new("phone").unwrap(),
        }),
    )
    .await;
    let ResultBody::Hello(replacement) = replacement.outcome.unwrap() else {
        panic!("replacement hello")
    };
    assert_eq!(replacement.device_id, hello.device_id);
    assert_eq!(replacement.boot_id, hello.boot_id);
    tokio::time::timeout(Duration::from_secs(5), recovered.closed())
        .await
        .unwrap();
    let snapshot = rpc(
        &mut send,
        &mut recv,
        RequestBody::HostSnapshot(HostSnapshotParams {}),
    )
    .await;
    let ResultBody::HostSnapshot(snapshot) = snapshot.outcome.unwrap() else {
        panic!("snapshot")
    };
    assert_eq!(snapshot.seq.0, 0);
    assert!(snapshot.sessions.is_empty());
    let unpair = rpc(
        &mut send,
        &mut recv,
        RequestBody::DeviceUnpair(DeviceUnpairParams {
            boot_id: hello.boot_id,
        }),
    )
    .await;
    assert!(matches!(unpair.outcome, Ok(ResultBody::DeviceUnpair(_))));
    assert!(security.devices().await.unwrap().is_empty());
    stop.send(true).unwrap();
    server.await.unwrap();
    goatr::close_endpoint(&host).await.unwrap();
    goatr::close_endpoint(&phone).await.unwrap();
}
#[tokio::test]
async fn admin_newline_and_cap_are_enforced_on_actual_owned_duplex() {
    use tokio::io::AsyncWriteExt;
    let (mut write, mut read) = tokio::io::duplex(1024);
    write.write_all(b"{}").await.unwrap();
    write.shutdown().await.unwrap();
    assert!(admin::read_line(&mut read).await.is_err());
    let (mut write, mut read) = tokio::io::duplex(1024);
    let writer = tokio::spawn(async move {
        let block = [b'x'; 1024];
        for _ in 0..65 {
            if write.write_all(&block).await.is_err() {
                return;
            }
        }
        let _ = write.write_all(b"\n").await;
    });
    assert!(admin::read_line(&mut read).await.is_err());
    drop(read);
    writer.await.unwrap();
}
