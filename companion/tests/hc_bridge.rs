// SPDX-License-Identifier: GPL-3.0-or-later
use goatr::{
    pairing::*,
    protocol::*,
    state::State,
    storage::{PairedDevice, Storage, StorageOwner},
    transport,
};
use std::{sync::Arc, time::Duration};

#[test]
fn legacy_pairing_never_implies_hc_authority_and_revocation_removes_it() {
    let mut storage = Storage::memory([42; 32], now_ms()).unwrap();
    let device = PairedDevice {
        device_id: Uuid::random(),
        endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[22; 32]).public().to_string()),
        device_name: Label::new("phone").unwrap(),
        paired_at: Uint(now_ms()),
        last_seen_at: None,
    };
    let epoch = storage.pair_device(&device).unwrap();
    let proof = storage
        .authenticated_hello(&device.device_id, epoch)
        .unwrap();
    assert!(storage.require_hc(&proof).is_err());
    assert_eq!(storage.pair_device_authority(&device, true).unwrap(), epoch);
    storage.require_hc(&proof).unwrap();
    storage.revoke_epoch(&device.device_id, now_ms()).unwrap();
    assert!(storage.require_hc(&proof).is_err());
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
        Duration::from_secs(10),
        transport::read_frame(recv, FRAME_MAX),
    )
    .await
    .unwrap()
    .unwrap();
    let Envelope::Response(reply) = decode_payload(&bytes, Some(method)).unwrap() else {
        panic!("not a response")
    };
    assert_eq!(reply.id, id);
    reply
}
fn hello() -> RequestBody {
    RequestBody::Hello(HelloParams {
        app_version: "hc-bridge-test".into(),
        device_name: Label::new("test phone").unwrap(),
    })
}

/// Real native integration, never an echo subprocess. Owns a fresh, empty core and all test state.
/// Run alone with GOATR_HC set to the pinned upstream hc executable.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires absolute GOATR_HC pointing to upstream hc 0.1.19"]
async fn real_hc_pair_stream_legacy_refusal_and_active_revocation() {
    use std::os::unix::fs::DirBuilderExt;
    let executable =
        std::path::PathBuf::from(std::env::var_os("GOATR_HC").expect("GOATR_HC required"));
    assert!(executable.is_absolute() && executable.is_file());
    let root = std::env::temp_dir().join(format!("ghc-{}", Uuid::random().0));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let socket = root.join("c.sock");
    // This test is invoked alone. Set process environment before starting endpoint/background jobs,
    // so the bridge's unchanged child and this owned core use the same private session, never defaults.
    unsafe {
        std::env::set_var("HC_SESSION", "goatr-bridge-test");
        std::env::set_var("HC_SOCKET", &socket);
        std::env::set_var("XDG_RUNTIME_DIR", &root);
        std::env::set_var("XDG_STATE_HOME", root.join("state"));
        std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    }
    let mut core = tokio::process::Command::new(&executable)
        .arg("serve")
        .current_dir(&root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        while !socket.exists() {
            assert!(core.try_wait().unwrap().is_none(), "owned core exited");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    // Real hc grants command authority, so these network identities must not be shared test keys.
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    let host = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&key))
        .alpns(vec![goatr::HC_ALPN.to_vec()])
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    getrandom::fill(&mut key).unwrap();
    let phone = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&key))
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    key.fill(0);
    let mut storage = Storage::memory([42; 32], now_ms()).unwrap();
    let boot = storage.boot_id.clone();
    let old = PairedDevice {
        device_id: Uuid::random(),
        endpoint_id: EndpointId(phone.id().to_string()),
        device_name: Label::new("legacy phone").unwrap(),
        paired_at: Uint(now_ms()),
        last_seen_at: None,
    };
    storage.pair_device(&old).unwrap();
    let security = Security::new(
        StorageOwner::new(storage).unwrap(),
        Arc::new(State::default()),
        EndpointId(host.id().to_string()),
        boot,
        vec![],
        None,
    );
    let (stop, rx) = tokio::sync::watch::channel(false);
    let endpoint = host.clone();
    let admission = security.clone();
    let server = tokio::spawn(async move {
        transport::serve(&endpoint, admission, None, None, None, rx)
            .await
            .unwrap()
    });
    let legacy = phone.connect(host.addr(), goatr::HC_ALPN).await.unwrap();
    let (mut send, mut recv) = legacy.open_bi().await.unwrap();
    assert_eq!(
        rpc(&mut send, &mut recv, hello())
            .await
            .outcome
            .unwrap_err()
            .code,
        ErrorCode::Unauthorized
    );
    legacy.close(0u32.into(), b"done");
    let invite = PairInvite::parse(&security.open_pair().await.unwrap().uri).unwrap();
    let connection = phone.connect(host.addr(), goatr::HC_ALPN).await.unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let paired = rpc(
        &mut send,
        &mut recv,
        RequestBody::PairConfirm(PairConfirmParams {
            token: invite.token,
            device_name: Label::new("full control phone").unwrap(),
        }),
    )
    .await;
    let ResultBody::PairConfirm(paired) = paired.outcome.unwrap() else {
        panic!("pair result")
    };
    assert_eq!(
        paired.device_id, old.device_id,
        "fresh consent upgrades the same retained identity"
    );
    assert!(rpc(&mut send, &mut recv, hello()).await.outcome.is_ok());
    send.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hello\",\"params\":{\"generation\":1,\"framed\":true}}\n").await.unwrap();
    let mut line = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let mut byte = [0];
            recv.read_exact(&mut byte).await.unwrap();
            if byte[0] == b'\n' {
                break;
            }
            assert!(line.len() < 262144);
            line.push(byte[0]);
        }
    })
    .await
    .unwrap();
    let hello: serde_json::Value = serde_json::from_slice(&line).unwrap();
    assert_eq!(hello["result"]["generation"], 1);
    assert_eq!(hello["result"]["framed"], true);
    assert_eq!(hello["result"]["version"], "0.1.19");
    let payload = br#"{"jsonrpc":"2.0","id":1,"method":"terminal.list","params":{}}"#;
    let mut frame = vec![0u8; 9];
    frame[5..9].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    send.write_all(&frame).await.unwrap();
    let mut header = [0u8; 9];
    tokio::time::timeout(Duration::from_secs(10), recv.read_exact(&mut header))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&header[..5], &[0; 5]);
    let size = u32::from_le_bytes(header[5..9].try_into().unwrap()) as usize;
    assert!(size < 262144);
    let mut body = vec![0; size];
    recv.read_exact(&mut body).await.unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["result"]["terminals"], serde_json::json!([]));
    security.revoke(paired.device_id, None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), connection.closed())
        .await
        .unwrap();
    stop.send(true).unwrap();
    server.await.unwrap();
    goatr::close_endpoint(&phone).await.unwrap();
    goatr::close_endpoint(&host).await.unwrap();
    // This handle is our unreaped original process, not a rediscovered pid or desktop process.
    core.kill().await.unwrap();
    core.wait().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
