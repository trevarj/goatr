// SPDX-License-Identifier: GPL-3.0-or-later
//! Consumer of the actual installed MainActivity/Compose and native-camera/IME tests.
use super::*;
use std::collections::BTreeMap;
use std::os::unix::fs::OpenOptionsExt;
use std::pin::Pin;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::watch;

const CONTROL_DOMAIN: &[u8] = b"GOATR-STOCK-CONTROL-v1\0";
#[derive(Clone, Copy)]
struct AppEpoch {
    process: (u32, u64),
    public: Option<[u8; 65]>,
}

struct InstrumentationEvidence<'a> {
    cleanup_admission: &'a watch::Sender<Option<AppEpoch>>,
    evidence: &'a Path,
    run: &'a Uuid,
    apk: &'a Path,
    apk_sha256: &'a str,
    test_apk: &'a Path,
    test_apk_sha256: &'a str,
}

impl InstrumentationEvidence<'_> {
    fn persist(&self, lease: &Lease, epoch: AppEpoch) -> io::Result<()> {
        write_json(
            &self.evidence.join(format!(
                "development-android-instrumentation-epoch-{}.json",
                self.run.0
            )),
            &json!({
                "runId":self.run,
                "emulatorLease":{
                    "runId":lease.run_id,
                    "ownerUid":lease.owner_uid,
                    "serial":lease.serial,
                    "avdName":lease.avd_name,
                    "adbClientHome":lease.adb_client_home,
                    "process":{
                        "linuxBootId":lease.process_identity.linux_boot_id,
                        "pid":lease.process_identity.pid,
                        "startTicks":lease.process_identity.start_ticks,
                        "executable":lease.process_identity.executable,
                    },
                },
                "originalApp":{"pid":epoch.process.0,"startTicks":epoch.process.1.to_string()},
                "apk":{"path":self.apk,"sha256":self.apk_sha256},
                "testApk":{"path":self.test_apk,"sha256":self.test_apk_sha256},
                "scope":"fresh original instrumentation epoch retained before control admission; no signing key or control endpoint retained",
            }),
        )
    }
}
async fn cancel_on_signal<T>(operation: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = operation => result,
        _ = tokio::signal::ctrl_c() => Err(failure("owned acceptance cancelled; potentially committed native work retained, never replayed or declared no-effect")),
        _ = terminate.recv() => Err(failure("owned acceptance cancelled; potentially committed native work retained, never replayed or declared no-effect")),
    }
}
fn control_message(challenge: &[u8; 32], bytes: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(CONTROL_DOMAIN.len() + 36 + bytes.len());
    message.extend_from_slice(CONTROL_DOMAIN);
    message.extend_from_slice(challenge);
    message.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    message.extend_from_slice(bytes);
    message
}
async fn authenticated_control(stream:&mut tokio::net::TcpStream,public:&[u8;65])->io::Result<Control> {
    authenticated_control_record(stream,public,None).await
}
pub(super) async fn authenticated_control_record(
    stream: &mut tokio::net::TcpStream,
    public: &[u8; 65],
    mut witness:Option<&mut Value>,
) -> io::Result<Control> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let challenge = random32()?;
        stream.write_all(&challenge).await?;
        let mut claimed = [0; 65];
        stream.read_exact(&mut claimed).await?;
        if &claimed != public {
            return Err(failure("unknown stock-control signing identity"));
        }
        let signature_len = stream.read_u8().await? as usize;
        if !(8..=72).contains(&signature_len) {
            return Err(failure("stock-control signature bound"));
        }
        let mut signature = [0; 72];
        stream.read_exact(&mut signature[..signature_len]).await?;
        let length = stream.read_u32().await? as usize;
        if !(1..=65536).contains(&length) {
            return Err(failure("stock-control request bound"));
        }
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).await?;
        let mut message = control_message(&challenge, &bytes);
        let verified = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_ASN1,
            public,
        )
        .verify(&message, &signature[..signature_len]);
        message.fill(0);
        let value = if verified.is_ok() {
            goatr::native_json::parse(&bytes, goatr::native_json::REGISTRY)
                .map_err(|_| failure("invalid authenticated stock-control JSON"))
        } else {
            Err(failure("stock-control signature rejected"))
        };
        if verified.is_ok() && let Some(record)=witness.as_deref_mut() {
            *record=json!({"public":hex(public),"challenge":hex(&challenge),"signature":hex(&signature[..signature_len]),
                "request":std::str::from_utf8(&bytes).map_err(io::Error::other)?});
        }
        bytes.fill(0);
        serde_json::from_value(value?)
            .map_err(|_| failure("closed authenticated stock-control shape"))
    })
    .await
    .map_err(|_| failure("stock-control authentication deadline"))?
}
#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Control {
    // Empty structs enforce a closed map; internally tagged unit variants ignore extra fields.
    PhysicalReport {
        #[serde(rename="runId")]run_id:Uuid,
        sequence:Uint,
        report:Value,
    },
    LivePair {},
    LiveInfo {},
    Observe {
        target: Target,
        marker: String,
        receipt: Receipt,
    },
    Older {
        target: Target,
    },
    Desktop {
        provider: Provider,
    },
    TriggerDialog {
        provider: Provider,
        kind: PresentationKind,
    },
    DialogClosed {
        target: Target,
        request_key: Uuid,
        receipt: Receipt,
    },
    Gap {
        device_id: Uuid,
    },
    Restart {},
    Revoke {
        device_id: Uuid,
    },
    Capture {
        scene: Scene,
    },
    VerifyCreate {
        receipt: Receipt,
    },
    FaultCreate {
        receipt: Receipt,
    },
    ProcessDeath {
        device_id: Uuid,
        action_id: Uuid,
        target: Target,
        expires_at: Uint,
    },
    RecoveryInfo {},
    VerifyNoReplay {
        action_id: Uuid,
        target: Target,
        receipt: Receipt,
    },
    Complete {},
    FeedbackPaired {
        device_id: Uuid,
    },
    FeedbackReady {
        device_id: Uuid,
        sessions: Vec<SessionSummary>,
        apk_sha256: String,
        test_apk_sha256: String,
    },
    DevelopmentReady {
        device_id: Uuid,
        sessions: Vec<SessionSummary>,
        #[serde(default)]
        history: Option<Vec<DevelopmentHistory>>,
    },
}
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DevelopmentHistory {
    provider: Provider,
    status: DevelopmentHistoryStatus,
}
#[derive(Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum DevelopmentHistoryStatus {
    Observed,
    Unproven,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Scene {
    Conversation,
    Older,
    Tools,
    Recovered,
    Creation,
    Offline,
}
impl Scene {
    fn name(&self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Older => "older",
            Self::Tools => "tools",
            Self::Recovered => "recovered",
            Self::Creation => "creation",
            Self::Offline => "offline",
        }
    }
}
pub(super) async fn adb_command(l: &Lease, args: &[&str]) -> io::Result<tokio::process::Command> {
    let sdk = sdk_adb().await?;
    if !emulator_matches(l) {
        return Err(failure("owned emulator changed before SDK command"));
    }
    let mut c = tokio::process::Command::new(sdk);
    super::configure_adb_client_home(&mut c,&l.root,&l.adb_client_home)?;
    c.args(super::adb_selector(l)?)
        .args(args)
        .kill_on_drop(true);
    Ok(c)
}
fn owned_package_present(output: &str, package: &str) -> io::Result<bool> {
    if !matches!(package, APP | "io.github.trevarj.goatr.debug.test") {
        return Err(failure("APK reset package is outside the owned scope"));
    }
    let expected = format!("package:{package}");
    let lines: Vec<_> = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    match lines.as_slice() {
        [] => Ok(false),
        [line] if *line == expected => Ok(true),
        _ => Err(failure("owned package presence was ambiguous")),
    }
}
async fn reset_owned_apks(l: &Lease) -> io::Result<()> {
    // Fresh acceptance owns only these two debug packages; never preserve a stale
    // signing identity or touch another package to make installation succeed.
    for package in ["io.github.trevarj.goatr.debug.test", APP] {
        let listed = adb(l, &["shell", "pm", "list", "packages", package]).await?;
        if !owned_package_present(&listed, package)? {
            continue;
        }
        if super::camera::app_process(l).await?.is_some() {
            return Err(failure("owned APK reset refuses a live prior app process"));
        }
        if adb(l, &["uninstall", package]).await?.trim() != "Success" {
            return Err(failure("owned APK uninstall was not confirmed; no retry"));
        }
        let listed = adb(l, &["shell", "pm", "list", "packages", package]).await?;
        if owned_package_present(&listed, package)? {
            return Err(failure("owned APK remains after uninstall; no retry"));
        }
    }
    Ok(())
}
async fn install_owned_apk(l: &Lease, apk: &Path, instrumentation: bool) -> io::Result<()> {
    let stage = if instrumentation {
        "instrumentation"
    } else {
        "app"
    };
    adb(l, &["install", "-r", apk.to_str().ok_or_else(|| failure("APK path not UTF-8"))?])
        .await
        .map_err(|error| {
            let detail = error.get_ref().and_then(|inner| inner.downcast_ref::<ApkInstallFailure>());
            io::Error::new(error.kind(), format!(
                "SDK apk_install stage={stage}; category={}; exit={:?}; io={:?}; raw output omitted",
                detail.map_or("other", |value| value.category),
                detail.and_then(|value| value.exit), error.kind()
            ))
        })?;
    Ok(())
}
pub(super) async fn instrument(
    l: &Lease,
    class: &str,
    port: Option<u16>,
    camera: bool,
    feed: Option<&mut super::camera::Feed<'_>>,
) -> io::Result<Value> {
    let (admission, admitted) = watch::channel(None);
    let result = cancel_on_signal(instrument_owned(
        l, class, port, camera, feed, &admission, None,
    ))
    .await;
    let epoch = *admitted.borrow();
    let cleanup = super::camera::stop_app(l, epoch.map(|e: AppEpoch| e.process)).await;
    preserve_instrumentation_error(result, cleanup.map(|_| ()))
}
fn preserve_instrumentation_error<T>(
    result: io::Result<T>,
    cleanup: io::Result<()>,
) -> io::Result<T> {
    match result {
        Err(error) => Err(error),
        Ok(value) => cleanup.map(|()| value),
    }
}
const INSTRUMENTATION_SELECTIONS: &[(&str, &str)] = &[
    (
        "io.github.trevarj.goatr.NativeLoadTest,io.github.trevarj.goatr.NativeInventoryTest",
        "initial_native_components",
    ),
    (
        "io.github.trevarj.goatr.SessionStoreTest#olderWindowRetainsRequestedRowsAcrossLiveUpdatesAndResetsOnlyOnSnapshot",
        "reducer_component",
    ),
    (
        "io.github.trevarj.goatr.ProviderFormsTest#actualAospImeVisibleKeysEnterBoundedNegativeDecimalAndInteger",
        "ime_component",
    ),
    (
        "io.github.trevarj.goatr.PairingUiTest#deniedCameraLeavesActualPasteAndDisposedQueuedCallbacksCannotAccept",
        "pairing_component",
    ),
    (
        "io.github.trevarj.goatr.PairingUiTest#realCameraFramesDecodeSyntheticInvitationAndBackReentryPasteNeverPair",
        "camera_component",
    ),
    (
        "io.github.trevarj.goatr.GoatrConnectionTest#stock_packaged_host_actual_main_activity_both_selected_providers",
        "stock_foreground",
    ),
    (
        "io.github.trevarj.goatr.GoatrConnectionTest#stock_process_death_restores_original_target_receipt_without_replay_then_forgets",
        "stock_process_death",
    ),
    (
        "io.github.trevarj.goatr.GoatrConnectionTest#stock_post_pass_fresh_pair_retains_both_original_sessions_for_feedback",
        "stock_feedback",
    ),
    (
        "io.github.trevarj.goatr.GoatrConnectionTest#development_fresh_sessions_continue_from_android_to_desktop",
        "development_foreground",
    ),
    (
        "io.github.trevarj.goatr.GoatrConnectionTest#development_feedback_android_to_desktop",
        "development_feedback",
    ),
    (
        "io.github.trevarj.goatr.GoatrConnectionTest#stock_background_persistent_real_native_both_providers",
        "stock_background_persistent",
    ),
    (
        "io.github.trevarj.goatr.GoatrConnectionTest#stock_background_ntfy_real_native_both_providers",
        "stock_background_ntfy",
    ),
];
#[derive(Clone, Debug, PartialEq, Eq)]
struct AutomationSnapshot {
    phase: &'static str,
    provider: &'static str,
    state: String,
}
impl AutomationSnapshot {
    fn parse(value: &str) -> Option<Self> {
        if value.len() > 384
            || !value.is_ascii()
            || value
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            return None;
        }
        let mut fields = value.split('|');
        let (phase, provider, state) = (fields.next()?, fields.next()?, fields.next()?);
        if fields.next().is_some() {
            return None;
        }
        if phase != "receipt_timeout" && value.len() > 256 {
            return None;
        }
        let provider = match provider {
            "omp" => "omp",
            "codex" => "codex",
            "none" => "none",
            _ => return None,
        };
        let provider_session = matches!(provider, "omp" | "codex");
        let ready_bits = || {
            let mut fields = state.split(',');
            ["selection", "reservation", "current", "send_enabled"]
                .into_iter()
                .all(
                    |name| match fields.next().and_then(|value| value.split_once('=')) {
                        Some((key, "0" | "1")) => key == name,
                        _ => false,
                    },
                )
                && fields.next().is_none()
        };
        let effect_state = || {
            let mut fields = state.split(',');
            ["rows", "calls", "unkeyed"].into_iter().all(|name| {
                match fields.next().and_then(|value| value.split_once('=')) {
                    Some((key, "zero" | "one" | "many")) => key == name,
                    _ => false,
                }
            }) && matches!(
                fields.next().and_then(|value| value.split_once('=')),
                Some(("identity", "same" | "changed" | "missing"))
            ) && fields.next().is_none()
        };
        let stock_await_timeout = || {
            let mut fields = state.split(',');
            [
                "hello",
                "inventory_ready",
                "source_present",
                "exact_reducer",
                "mutations_enabled",
                "busy",
                "refresh_enabled",
                "subscription_present",
                "exact_selection",
            ]
            .into_iter()
            .all(|name| {
                matches!(
                    fields.next().and_then(|value| value.split_once('=')),
                    Some((key, "0" | "1")) if key == name
                )
            }) && matches!(
                fields.next().and_then(|value| value.split_once('=')),
                Some((
                    "connectivity",
                    "ready"
                        | "synchronizing"
                        | "offline"
                        | "setup_required"
                        | "unavailable"
                        | "absent"
                ))
            ) && matches!(
                fields.next().and_then(|value| value.split_once('=')),
                Some(("inventory_target", "same" | "replaced" | "absent"))
            ) && fields.next().is_none()
        };
        let send_admission = || {
            let mut fields = state.split(',');
            matches!(
                fields.next().and_then(|value| value.split_once('=')),
                Some(("point", "before" | "after"))
            ) && matches!(
                fields.next().and_then(|value| value.split_once('=')),
                Some(("reservation", "same" | "other" | "absent"))
            ) && fields.next().is_none()
        };
        let refresh_timeout = || {
            let mut fields = state.split(',');
            [
                "selection",
                "busy",
                "hello",
                "inventory_ready",
                "mutation_ready",
            ]
            .into_iter()
            .all(|name| {
                matches!(
                    fields.next().and_then(|value| value.split_once('=')),
                    Some((key, "0" | "1")) if key == name
                )
            }) && matches!(
                fields.next().and_then(|value| value.split_once('=')),
                Some(("source", "absent" | "same" | "changed" | "unobserved"))
            ) && matches!(
                fields.next().and_then(|value| value.split_once('=')),
                Some(("reducer", "present" | "absent"))
            ) && matches!(
                fields.next().and_then(|value| value.split_once('=')),
                Some(("older", "true" | "false" | "unobserved"))
            ) && matches!(
                fields.next().and_then(|value| value.split_once('=')),
                Some((
                    "notice",
                    "none"
                        | "recovery_approval"
                        | "protected_key"
                        | "generic"
                        | "other"
                        | "store_policy:InvalidRequest"
                        | "store_policy:UnsupportedVersion"
                        | "store_policy:UnsupportedMethod"
                        | "store_policy:Unauthorized"
                        | "store_policy:PairingFailed"
                        | "store_policy:StaleHost"
                        | "store_policy:StaleSession"
                        | "store_policy:NotFound"
                        | "store_policy:NotReady"
                        | "store_policy:InvalidAnswer"
                        | "store_policy:AlreadyResolved"
                        | "store_policy:ActionIdConflict"
                        | "store_policy:ActionCapacity"
                        | "store_policy:ContentExpired"
                        | "store_policy:CursorExpired"
                        | "store_policy:Busy"
                        | "store_policy:ResourceLimit"
                        | "store_policy:RateLimited"
                        | "store_policy:Timeout"
                        | "store_policy:BackendError"
                        | "store_policy:PushRejected"
                        | "store_policy:StaleRegistration"
                        | "remote_failure:InvalidRequest"
                        | "remote_failure:UnsupportedVersion"
                        | "remote_failure:UnsupportedMethod"
                        | "remote_failure:Unauthorized"
                        | "remote_failure:PairingFailed"
                        | "remote_failure:StaleHost"
                        | "remote_failure:StaleSession"
                        | "remote_failure:NotFound"
                        | "remote_failure:NotReady"
                        | "remote_failure:InvalidAnswer"
                        | "remote_failure:AlreadyResolved"
                        | "remote_failure:ActionIdConflict"
                        | "remote_failure:ActionCapacity"
                        | "remote_failure:ContentExpired"
                        | "remote_failure:CursorExpired"
                        | "remote_failure:Busy"
                        | "remote_failure:ResourceLimit"
                        | "remote_failure:RateLimited"
                        | "remote_failure:Timeout"
                        | "remote_failure:BackendError"
                        | "remote_failure:PushRejected"
                        | "remote_failure:StaleRegistration"
                ))
            ) && matches!(
                fields.next().and_then(|value| value.split_once('=')),
                Some(("failure", "compose_timeout" | "cancelled" | "other"))
            ) && fields.next().is_none()
        };
        let receipt_timeout = || {
            let mut fields = state.split(',');
            ["foreground", "busy", "exact_selection", "current"]
                .into_iter()
                .all(|name| {
                    matches!(
                        fields.next().and_then(|value| value.split_once('=')),
                        Some((key, "0" | "1")) if key == name
                    )
                })
                && [
                    "before_reservation",
                    "after_reservation",
                    "reservation",
                    "replacement",
                    "focused",
                    "local_row",
                ]
                .into_iter()
                .all(|name| {
                    matches!(
                        fields.next().and_then(|value| value.split_once('=')),
                        Some((key, "same" | "other" | "absent")) if key == name
                    )
                })
                && matches!(
                    fields.next().and_then(|value| value.split_once('=')),
                    Some((
                        "notice",
                        "none"
                            | "recovery_approval"
                            | "protected_key"
                            | "generic"
                            | "other"
                            | "store_policy:InvalidRequest"
                            | "store_policy:UnsupportedVersion"
                            | "store_policy:UnsupportedMethod"
                            | "store_policy:Unauthorized"
                            | "store_policy:PairingFailed"
                            | "store_policy:StaleHost"
                            | "store_policy:StaleSession"
                            | "store_policy:NotFound"
                            | "store_policy:NotReady"
                            | "store_policy:InvalidAnswer"
                            | "store_policy:AlreadyResolved"
                            | "store_policy:ActionIdConflict"
                            | "store_policy:ActionCapacity"
                            | "store_policy:ContentExpired"
                            | "store_policy:CursorExpired"
                            | "store_policy:Busy"
                            | "store_policy:ResourceLimit"
                            | "store_policy:RateLimited"
                            | "store_policy:Timeout"
                            | "store_policy:BackendError"
                            | "store_policy:PushRejected"
                            | "store_policy:StaleRegistration"
                            | "remote_failure:InvalidRequest"
                            | "remote_failure:UnsupportedVersion"
                            | "remote_failure:UnsupportedMethod"
                            | "remote_failure:Unauthorized"
                            | "remote_failure:PairingFailed"
                            | "remote_failure:StaleHost"
                            | "remote_failure:StaleSession"
                            | "remote_failure:NotFound"
                            | "remote_failure:NotReady"
                            | "remote_failure:InvalidAnswer"
                            | "remote_failure:AlreadyResolved"
                            | "remote_failure:ActionIdConflict"
                            | "remote_failure:ActionCapacity"
                            | "remote_failure:ContentExpired"
                            | "remote_failure:CursorExpired"
                            | "remote_failure:Busy"
                            | "remote_failure:ResourceLimit"
                            | "remote_failure:RateLimited"
                            | "remote_failure:Timeout"
                            | "remote_failure:BackendError"
                            | "remote_failure:PushRejected"
                            | "remote_failure:StaleRegistration"
                    ))
                )
                && match fields.next().and_then(|value| value.split_once('=')) {
                    Some(("admission", "none")) => true,
                    Some(("admission", value)) => {
                        let mut admission = value.split(':');
                        let phase = admission.next();
                        let guard = admission.next();
                        let code = admission.next();
                        let sent = admission.next();
                        let frame_started = admission.next();
                        let known_code = matches!(
                            code,
                            Some(
                                "InvalidRequest"
                                    | "UnsupportedVersion"
                                    | "UnsupportedMethod"
                                    | "Unauthorized"
                                    | "PairingFailed"
                                    | "StaleHost"
                                    | "StaleSession"
                                    | "NotFound"
                                    | "NotReady"
                                    | "InvalidAnswer"
                                    | "AlreadyResolved"
                                    | "ActionIdConflict"
                                    | "ActionCapacity"
                                    | "ContentExpired"
                                    | "CursorExpired"
                                    | "Busy"
                                    | "ResourceLimit"
                                    | "RateLimited"
                                    | "Timeout"
                                    | "BackendError"
                                    | "PushRejected"
                                    | "StaleRegistration"
                            )
                        );
                        admission.next().is_none()
                            && match (phase, guard, code, sent, frame_started) {
                                (
                                    Some("pre_presend" | "post_presend"),
                                    Some("boot_changed"),
                                    Some("StaleHost"),
                                    Some("unknown"),
                                    Some("unknown"),
                                )
                                | (
                                    Some("pre_presend" | "post_presend"),
                                    Some("target_changed"),
                                    Some("StaleSession"),
                                    Some("unknown"),
                                    Some("unknown"),
                                )
                                | (
                                    Some("pre_presend" | "post_presend"),
                                    Some(
                                        "host_not_ready"
                                        | "receipts_unreconciled"
                                        | "reducer_absent"
                                        | "summary_absent"
                                        | "mutations_disabled"
                                        | "source_not_current",
                                    ),
                                    Some("NotReady"),
                                    Some("unknown"),
                                    Some("unknown"),
                                )
                                | (
                                    Some("original_scope"),
                                    Some(
                                        "original_absent"
                                        | "original_not_alive"
                                        | "original_unauthenticated",
                                    ),
                                    Some("NotReady"),
                                    Some("unknown"),
                                    Some("unknown"),
                                )
                                | (
                                    Some("pre_presend" | "post_presend"),
                                    Some(
                                        "original_replaced"
                                        | "original_not_alive"
                                        | "original_unauthenticated",
                                    ),
                                    Some("NotReady"),
                                    Some("unknown"),
                                    Some("unknown"),
                                )
                                | (
                                    Some("request"),
                                    Some("not_alive" | "unauthenticated"),
                                    Some("Unauthorized"),
                                    Some("unknown"),
                                    Some("unknown"),
                                )
                                | (
                                    Some("request"),
                                    Some("inflight_busy"),
                                    Some("Busy"),
                                    Some("unknown"),
                                    Some("unknown"),
                                )
                                | (
                                    Some("request"),
                                    Some("pending_conflict"),
                                    Some("ActionIdConflict"),
                                    Some("unknown"),
                                    Some("unknown"),
                                )
                                | (
                                    Some("writer"),
                                    Some(
                                        "original_replaced"
                                        | "original_not_alive"
                                        | "original_unauthenticated",
                                    ),
                                    Some("NotReady"),
                                    Some("0"),
                                    Some("0"),
                                )
                                | (
                                    Some("writer"),
                                    Some("boot_changed"),
                                    Some("StaleHost"),
                                    Some("0"),
                                    Some("0"),
                                )
                                | (
                                    Some("writer"),
                                    Some("target_changed"),
                                    Some("StaleSession"),
                                    Some("0"),
                                    Some("0"),
                                )
                                | (
                                    Some("writer"),
                                    Some(
                                        "host_not_ready"
                                        | "receipts_unreconciled"
                                        | "reducer_absent"
                                        | "summary_absent"
                                        | "mutations_disabled"
                                        | "source_not_current"
                                        | "not_alive"
                                        | "io_before_frame",
                                    ),
                                    Some("NotReady"),
                                    Some("0"),
                                    Some("0"),
                                )
                                | (
                                    Some("writer"),
                                    Some("io_after_frame_start"),
                                    Some("NotReady"),
                                    Some("0"),
                                    Some("1"),
                                )
                                | (
                                    Some("pending"),
                                    Some("connection_closed"),
                                    Some("NotReady"),
                                    Some("0" | "1"),
                                    Some("unknown"),
                                )
                                | (
                                    Some("response_apply"),
                                    Some("other"),
                                    Some("NotReady"),
                                    Some("0" | "1"),
                                    Some("unknown"),
                                )
                                | (
                                    Some("existing_status"),
                                    Some("unknown"),
                                    Some("NotReady"),
                                    Some("unknown"),
                                    Some("unknown"),
                                ) => true,
                                (
                                    Some("response_apply"),
                                    Some("store_policy"),
                                    _,
                                    Some("0" | "1"),
                                    Some("unknown"),
                                ) => known_code,
                                _ => false,
                            }
                    }
                    _ => false,
                }
                && fields.next().is_none()
        };
        let valid = match phase {
            "pairing" => {
                provider == "none" && matches!(state, "startup" | "begin" | "preview" | "confirmed")
            }
            "session_open" => provider_session && matches!(state, "requested" | "ready"),
            "history_attempt" => {
                provider_session && matches!(state, "requested" | "observed" | "unproven")
            }
            "refresh" => provider_session && matches!(state, "requested" | "current"),
            "refresh_timeout" => provider_session && refresh_timeout(),
            "stock_await_timeout" => provider_session && stock_await_timeout(),
            "send_wait" => provider_session && ready_bits(),
            "send_ready" => provider_session && state == "selected_reserved_current_enabled",
            "send_admission" => provider_session && send_admission(),
            "send_clicked" => provider_session && state == "clicked",
            "receipt_polling" => {
                provider_session
                    && matches!(
                        state.split_once(",authoritative="),
                        Some((
                            "state=absent"
                                | "state=reserved"
                                | "state=dispatching"
                                | "state=dispatched"
                                | "state=succeeded"
                                | "state=failed"
                                | "state=uncertain",
                            "0" | "1"
                        ))
                    )
            }
            "receipt_timeout" => provider_session && receipt_timeout(),
            "receipt_display" => provider_session && state == "canonical_recovery_cleared",
            "native_observe" | "desktop" => {
                provider_session && matches!(state, "requested" | "verified" | "observed")
            }
            "tool_history" => {
                provider_session && (matches!(state, "awaiting" | "observed") || effect_state())
            }
            "reconnect" => {
                (provider_session
                    && (matches!(state, "requested" | "receipt_verified") || effect_state()))
                    || (provider == "none" && matches!(state, "background" | "resume_wait"))
            }
            "cleanup" => provider == "none" && matches!(state, "normal" | "primary_failed"),
            "cleanup_failure" => provider == "none" && state == "closed",
            _ => false,
        };
        valid.then(|| Self {
            phase: match phase {
                "pairing" => "pairing",
                "session_open" => "session_open",
                "history_attempt" => "history_attempt",
                "refresh" => "refresh",
                "refresh_timeout" => "refresh_timeout",
                "stock_await_timeout" => "stock_await_timeout",
                "send_wait" => "send_wait",
                "send_ready" => "send_ready",
                "send_admission" => "send_admission",
                "send_clicked" => "send_clicked",
                "receipt_polling" => "receipt_polling",
                "receipt_timeout" => "receipt_timeout",
                "receipt_display" => "receipt_display",
                "native_observe" => "native_observe",
                "tool_history" => "tool_history",
                "desktop" => "desktop",
                "reconnect" => "reconnect",
                "cleanup" => "cleanup",
                "cleanup_failure" => "cleanup_failure",
                _ => unreachable!(),
            },
            provider,
            state: state.to_owned(),
        })
    }
    fn cleanup(&self) -> bool {
        matches!(self.phase, "cleanup" | "cleanup_failure")
    }
}
#[derive(Debug)]
struct InstrumentationFailureContext {
    selected: &'static str,
    consumer: &'static str,
    class: Option<&'static str>,
    method: Option<&'static str>,
    location: Option<(&'static str, u32)>,
    category: &'static str,
    status_codes: Vec<i32>,
    result_code: Option<i32>,
    statuses_truncated: bool,
    failure_seen: bool,
    exit_code: Option<i32>,
    automation_pending: Option<AutomationSnapshot>,
    automation_last: Option<AutomationSnapshot>,
    automation_first_failure: Option<AutomationSnapshot>,
    automation_seen: u8,
    automation_truncated: bool,
    automation_cleanup_failed: bool,
    automation_effects: BTreeMap<&'static str, String>,
    automation_receipt_display: BTreeMap<&'static str, String>,
    automation_receipt_display_invalid: bool,
}
impl InstrumentationFailureContext {
    fn new(selected: &str) -> Self {
        let (selected, consumer) = INSTRUMENTATION_SELECTIONS
            .iter()
            .copied()
            .find(|(known, _)| *known == selected)
            .unwrap_or(("other", "other"));
        Self {
            selected,
            consumer,
            class: None,
            method: None,
            location: None,
            category: "other",
            status_codes: Vec::new(),
            result_code: None,
            statuses_truncated: false,
            failure_seen: false,
            exit_code: None,
            automation_pending: None,
            automation_last: None,
            automation_first_failure: None,
            automation_seen: 0,
            automation_truncated: false,
            automation_cleanup_failed: false,
            automation_effects: BTreeMap::new(),
            automation_receipt_display: BTreeMap::new(),
            automation_receipt_display_invalid: false,
        }
    }
    fn accepts_class(&self, class: &str) -> bool {
        self.selected
            .split(',')
            .any(|selected| selected.split('#').next() == Some(class))
    }
    fn number(value: &str) -> Option<i32> {
        let value = value.trim();
        if value.len() > 5 {
            return None;
        }
        value
            .parse::<i32>()
            .ok()
            .filter(|code| (-999..=999).contains(code))
    }
    fn commit_automation(&mut self) {
        let Some(snapshot) = self.automation_pending.take() else {
            return;
        };
        if self.automation_seen >= 64 {
            self.automation_truncated = true;
            return;
        }
        self.automation_seen += 1;
        if snapshot.phase == "cleanup_failure" {
            self.automation_cleanup_failed = true;
        }
        if snapshot.phase == "reconnect" && snapshot.state.starts_with("rows=") {
            self.automation_effects
                .insert(snapshot.provider, snapshot.state.clone());
        }
        if snapshot.phase == "receipt_display"
            && self
                .automation_receipt_display
                .insert(snapshot.provider, snapshot.state.clone())
                .is_some()
        {
            self.automation_receipt_display_invalid = true;
        }
        if !snapshot.cleanup() {
            self.automation_last = Some(snapshot);
        }
    }
    fn observe(&mut self, line: &str) {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("INSTRUMENTATION_STATUS: goatrAutomationSnapshot=") {
            if self.automation_pending.is_none() {
                self.automation_pending = AutomationSnapshot::parse(value);
            }
            return;
        }
        if let Some(value) = line.strip_prefix("INSTRUMENTATION_STATUS_CODE:") {
            if let Some(code) = Self::number(value) {
                self.commit_automation();
                if self.status_codes.len() < 32 {
                    self.status_codes.push(code);
                } else {
                    self.statuses_truncated = true;
                }
                if matches!(code, -1 | -2) {
                    self.failure_seen = true;
                    if self.automation_first_failure.is_none() {
                        self.automation_first_failure = self.automation_last.clone();
                    }
                } else if code == 1 && !self.failure_seen {
                    self.location = None;
                    self.category = "other";
                }
            }
            return;
        }
        if let Some(value) = line.strip_prefix("INSTRUMENTATION_CODE:") {
            self.result_code = Self::number(value);
            return;
        }
        if self.failure_seen {
            return;
        }
        if let Some(value) = line.strip_prefix("INSTRUMENTATION_STATUS: class=") {
            self.class = [
                "NativeLoadTest",
                "NativeInventoryTest",
                "SessionStoreTest",
                "ProviderFormsTest",
                "PairingUiTest",
                "GoatrConnectionTest",
            ]
            .into_iter()
            .find(|class| {
                value.strip_prefix("io.github.trevarj.goatr.") == Some(*class)
                    && self.accepts_class(value)
            });
        }
        if let Some(value) = line.strip_prefix("INSTRUMENTATION_STATUS: test=") {
            self.method = [
                "realBindingLoadsAndRetainedEndpointIdentitySurvivesShutdown",
                "actualHierarchyKeepsNativeFocusDecorationsSeparateAndStaleAssociationsReadOnly",
                "noProjectsAndUnavailableNativeSourceDoNotOfferCreationAndPartialResourcesRemainVisible",
                "threeInspectedCreationResetsKeepTheMountedOwnedForm",
            ].into_iter().find(|known| *known == value && self.consumer == "initial_native_components")
                .or_else(|| self.selected.split_once('#').map(|(_, method)| method).filter(|method| *method == value));
        }
        let message = line
            .strip_prefix("INSTRUMENTATION_STATUS: stack=")
            .unwrap_or(line);
        if self.category == "other" {
            if message == "java.lang.AssertionError"
                || message.starts_with("java.lang.AssertionError:")
            {
                self.category = "assertion";
            } else if message.starts_with("org.junit.ComparisonFailure:") {
                self.category = "comparison";
            } else if message.starts_with("kotlinx.coroutines.TimeoutCancellationException:") {
                self.category = "timeout";
            } else if message.starts_with("INSTRUMENTATION_FAILED:") {
                self.category = "instrumentation_failed";
            }
        }
        if self.location.is_some() {
            return;
        }
        let Some(frame) = line.strip_prefix("at io.github.trevarj.goatr.") else {
            return;
        };
        let Some((owner, position)) = frame.split_once('(') else {
            return;
        };
        let Some(position) = position.strip_suffix(')') else {
            return;
        };
        let Some((file, number)) = position.split_once(':') else {
            return;
        };
        let owner = owner.split(['.', '$']).next().unwrap_or("");
        let known = [
            "NativeLoadTest.kt",
            "NativeInventoryTest.kt",
            "SessionStoreTest.kt",
            "ProviderFormsTest.kt",
            "PairingUiTest.kt",
            "GoatrConnectionTest.kt",
        ]
        .into_iter()
        .find(|known| *known == file && known.strip_suffix(".kt") == Some(owner));
        if !self.accepts_class(&format!("io.github.trevarj.goatr.{owner}")) || number.len() > 6 {
            return;
        }
        if let Some(file) = known
            && let Ok(number) = number.parse::<u32>()
            && (1..=999_999).contains(&number)
        {
            self.location = Some((file, number));
        }
    }
}
impl std::fmt::Display for InstrumentationFailureContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "actual installed instrumentation assertions failed; consumer={}; selected={}; class={:?}; method={:?}; location={:?}; category={}; status_codes={:?}; result_code={:?}; statuses_truncated={}; automation_last={:?}; automation_first_failure={:?}; automation_truncated={}; automation_cleanup_failed={}; exit_code={:?}; raw diagnostics omitted; no fixture fallback",
            self.consumer,
            self.selected,
            self.class,
            self.method,
            self.location,
            self.category,
            self.status_codes,
            self.result_code,
            self.statuses_truncated,
            self.automation_last,
            self.automation_first_failure,
            self.automation_truncated,
            self.automation_cleanup_failed,
            self.exit_code
        )
    }
}
impl std::error::Error for InstrumentationFailureContext {}
async fn finish_owned_instrumentation_with_deadline(
    child: &mut tokio::process::Child,
    deadline: Duration,
) -> io::Result<std::process::ExitStatus> {
    match tokio::time::timeout(deadline, child.wait()).await {
        Ok(status) => status,
        Err(_) => {
            child
                .start_kill()
                .map_err(|_| failure("owned instrumentation completion termination failed"))?;
            tokio::time::timeout(deadline, child.wait())
                .await
                .map_err(|_| failure("owned instrumentation completion teardown deadline"))?
        }
    }
}

async fn finish_owned_instrumentation(
    child: &mut tokio::process::Child,
) -> io::Result<std::process::ExitStatus> {
    finish_owned_instrumentation_with_deadline(child, Duration::from_secs(5)).await
}

enum InstrumentationControl<T> {
    Instrumentation(io::Result<Value>),
    Handler(io::Result<T>),
}

async fn poll_instrumentation_control<T, F, H>(
    mut child: Pin<&mut F>,
    handler: H,
) -> InstrumentationControl<T>
where
    F: Future<Output = io::Result<Value>>,
    H: Future<Output = io::Result<T>>,
{
    tokio::select! {
        result = child.as_mut() => InstrumentationControl::Instrumentation(result),
        result = handler => InstrumentationControl::Handler(result),
    }
}
async fn instrument_owned(
    l: &Lease,
    class: &str,
    port: Option<u16>,
    camera: bool,
    mut feed: Option<&mut super::camera::Feed<'_>>,
    admission: &watch::Sender<Option<AppEpoch>>,
    context: Option<&InstrumentationEvidence<'_>>,
) -> io::Result<Value> {
    if !emulator_matches(l) {
        return Err(failure(
            "native instrumentation refuses changed emulator owner",
        ));
    }
    let prior = super::camera::app_process(l).await?;
    if prior.is_some() {
        return Err(prerequisite(
            "instrumentation refuses an existing/unowned debug target process",
        ));
    }
    if admission.borrow().is_some() {
        return Err(failure(
            "instrumentation epoch must be explicitly cleared before replacement",
        ));
    }
    let component = format!("{APP}.test/io.github.trevarj.goatr.OwnedInstrumentationRunner");
    let mut c = adb_command(
        l,
        &[
            "shell",
            "am",
            "instrument",
            "-w",
            "-r",
            "-e",
            "class",
            class,
        ],
    )
    .await?;
    if let Some(port) = port {
        c.args(["-e", "stockControlPort", &port.to_string()]);
    }
    c.arg(&component)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = c.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failure("instrumentation status stream missing"))?;
    let mut lines = BufReader::new(stdout);
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(if camera { 180 } else { 1800 });
    let mut statuses = Vec::new();
    let mut failure_context = InstrumentationFailureContext::new(class);
    let mut success = false;
    let mut bytes = 0usize;
    let mut owner_state = None;
    let mut owner_pid = None;
    let mut owner_start = None;
    let mut policy = None;
    let mut policy_user = None;
    let mut camera_blocked = None;
    let mut camera_failure_stage = None;
    let mut camera_failure_class = None;
    let mut phase = None;
    let mut pid = None;
    let mut start = None;
    let mut public = None;
    let read_result: io::Result<()> = async {
    loop {
        let mut line = String::new();
        let n = tokio::time::timeout_at(deadline, (&mut lines).take(65537).read_line(&mut line))
            .await
            .map_err(|_| failure("actual installed instrumentation deadline"))??;
        if n == 0 {
            break;
        }
        if n > 65536 {
            return Err(failure("instrumentation status line exceeded bound"));
        }
        bytes += n;
        if bytes > 2 * 1024 * 1024 {
            return Err(failure("instrumentation diagnostics exceeded bound"));
        }
        failure_context.observe(&line);
        for (prefix, slot) in [
            ("INSTRUMENTATION_STATUS: goatrOwner=", &mut phase),
            ("INSTRUMENTATION_STATUS: goatrPid=", &mut pid),
            ("INSTRUMENTATION_STATUS: goatrStart=", &mut start),
            ("INSTRUMENTATION_STATUS: goatrControlPublic=", &mut public),
        ] {
            if let Some(value) = line.trim().strip_prefix(prefix)
                && slot.replace(value.to_owned()).is_some()
            {
                return Err(failure("duplicate instrumentation ownership status"));
            }
        }
        if line.starts_with("INSTRUMENTATION_STATUS_CODE:") && phase.is_some() {
            if phase.take().as_deref() != Some("fresh-start") || admission.borrow().is_some() {
                return Err(failure(
                    "unexpected/repeated original instrumentation epoch",
                ));
            }
            let process = (
                pid.take()
                    .and_then(|p| p.parse::<u32>().ok())
                    .filter(|p| *p > 0)
                    .ok_or_else(|| failure("original instrumentation PID missing"))?,
                start
                    .take()
                    .and_then(|p| p.parse::<u64>().ok())
                    .filter(|p| *p > 0)
                    .ok_or_else(|| failure("original instrumentation start missing"))?,
            );
            let public = match public.take() {
                Some(key) if port.is_some() && key.len() == 130 => {
                    let mut bytes = [0; 65];
                    for (i, pair) in key.as_bytes().as_chunks::<2>().0.iter().enumerate() {
                        let pair = std::str::from_utf8(pair).map_err(io::Error::other)?;
                        bytes[i] = u8::from_str_radix(pair, 16)
                            .map_err(|_| failure("invalid instrumentation public identity"))?;
                    }
                    if bytes[0] != 4 || hex(&bytes) != key {
                        return Err(failure("noncanonical instrumentation public identity"));
                    }
                    Some(bytes)
                }
                None if port.is_none() => None,
                _ => {
                    return Err(failure(
                        "original instrumentation signing identity missing/unexpected",
                    ));
                }
            };
            if super::camera::app_process(l).await? != Some(process) {
                return Err(failure(
                    "owned instrumentation stream PID/start does not join original target",
                ));
            }
            let epoch = AppEpoch { process, public };
            if let Some(context) = context {
                if context.cleanup_admission.borrow().is_some() {
                    return Err(failure(
                        "instrumentation cleanup epoch must be explicitly cleared before replacement",
                    ));
                }
                // Retain the proven epoch for cleanup even if the exclusive evidence write fails.
                context.cleanup_admission.send_replace(Some(epoch));
                context.persist(l, epoch)?;
            }
            admission.send_replace(Some(epoch));
        }
        if let Some(phase) = line
            .trim()
            .strip_prefix("INSTRUMENTATION_STATUS: goatrCameraPhase=")
        {
            if !camera
                || ![
                    "first-qr",
                    "neutral-after-back",
                    "policy-enable",
                    "policy-restore",
                    "second-qr",
                    "policy-original",
                ]
                .contains(&phase)
            {
                return Err(failure("unexpected camera native phase"));
            }
            feed.as_mut()
                .ok_or_else(|| failure("native camera phase requires the actual owned producer"))?
                .phase(phase)
                .await?;
        }
        if camera {
            for (prefix, slot) in [
                (
                    "INSTRUMENTATION_STATUS: goatrCameraFailureStage=",
                    &mut camera_failure_stage,
                ),
                (
                    "INSTRUMENTATION_STATUS: goatrCameraFailureClass=",
                    &mut camera_failure_class,
                ),
            ] {
                if let Some(value) = line.trim().strip_prefix(prefix)
                    && slot.replace(value.to_owned()).is_some()
                {
                    return Err(failure("duplicate native camera failure status field"));
                }
            }
            if line.starts_with("INSTRUMENTATION_STATUS_CODE:")
                && (camera_failure_stage.is_some() || camera_failure_class.is_some())
            {
                let stage = camera_failure_stage
                    .take()
                    .ok_or_else(|| failure("native camera failure stage missing"))?;
                let exception_class = camera_failure_class
                    .take()
                    .ok_or_else(|| failure("native camera failure class missing"))?;
                feed.as_mut()
                    .ok_or_else(|| failure("native camera failure lacks owned producer"))?
                    .failure_status(&stage, exception_class)?;
            }
            if let Some(value) = line
                .trim()
                .strip_prefix("INSTRUMENTATION_STATUS: goatrCameraPolicy=")
            {
                policy = match value {
                    "enabled" => Some(true),
                    "disabled" => Some(false),
                    _ => return Err(failure("invalid actual native policy capture")),
                };
            }
            if let Some(user) = line
                .trim()
                .strip_prefix("INSTRUMENTATION_STATUS: goatrCameraUser=")
            {
                policy_user = user.parse::<u32>().ok();
            }
            if let Some(reason) = line
                .trim()
                .strip_prefix("INSTRUMENTATION_STATUS: goatrCameraPolicyResult=")
            {
                camera_blocked = Some(match reason {
                    "capture-unavailable" => "actual native policy capture unavailable",
                    "unsupported-error-trigger" => {
                        "sensor privacy did not generate native CameraState error; black frames are not async-error proof"
                    }
                    _ => return Err(failure("unknown native camera prerequisite result")),
                });
            }
            if line.starts_with("INSTRUMENTATION_STATUS_CODE:") && policy.is_some() {
                feed.as_mut()
                    .ok_or_else(|| failure("actual policy capture without owned producer"))?
                    .policy_capture(
                        policy
                            .take()
                            .ok_or_else(|| failure("policy capture missing"))?,
                        policy_user
                            .take()
                            .ok_or_else(|| failure("policy guest user missing"))?,
                    )
                    .await?;
            }
            if let Some(state) = line
                .trim()
                .strip_prefix("INSTRUMENTATION_STATUS: goatrCameraOwner=")
            {
                owner_state = Some(state.to_owned());
            }
            if let Some(pid) = line
                .trim()
                .strip_prefix("INSTRUMENTATION_STATUS: goatrCameraPid=")
            {
                owner_pid = pid.parse::<u32>().ok();
            }
            if let Some(start) = line
                .trim()
                .strip_prefix("INSTRUMENTATION_STATUS: goatrCameraStart=")
            {
                owner_start = start.parse::<u64>().ok();
            }
            if line.starts_with("INSTRUMENTATION_STATUS_CODE:") && owner_state.is_some() {
                feed.as_mut()
                    .ok_or_else(|| failure("native owner status without camera producer"))?
                    .owner_status(
                        owner_state
                            .take()
                            .as_deref()
                            .ok_or_else(|| failure("native owner state missing"))?,
                        owner_pid
                            .take()
                            .ok_or_else(|| failure("native owner PID missing"))?,
                        owner_start
                            .take()
                            .ok_or_else(|| failure("native owner start missing"))?,
                    )
                    .await?;
            }
        }
        if (line.starts_with("INSTRUMENTATION_STATUS_CODE:")
            || line.starts_with("INSTRUMENTATION_CODE:"))
            && statuses.len() < 32
        {
            let prefix = if line.starts_with("INSTRUMENTATION_STATUS_CODE:") {
                "INSTRUMENTATION_STATUS_CODE:"
            } else {
                "INSTRUMENTATION_CODE:"
            };
            if let Some(code) = line
                .strip_prefix(prefix)
                .and_then(InstrumentationFailureContext::number)
            {
                statuses.push(format!("{prefix} {code}"));
            }
        }
        if line.contains("OK (") {
            success = true;
        }
        if line.contains("FAILURES!!!") || line.starts_with("INSTRUMENTATION_FAILED") {
            success = false;
        }
        // No instrumentation stream/semantics/control original, capability or secret is persisted.
        line.clear();
    }
        Ok(())
    }.await;
    drop(lines);
    let exited = finish_owned_instrumentation(&mut child).await;
    read_result?;
    if let Some(reason) = camera_blocked {
        return Err(prerequisite(reason));
    }
    let exited = exited?;
    if admission.borrow().is_none() {
        return Err(failure(
            "instrumentation completed without original-process rendezvous",
        ));
    }
    if !exited.success() || !success {
        if camera
            && let Some((stage, exception_class)) =
                feed.as_ref().and_then(|producer| producer.failure())
        {
            return Err(io::Error::other(format!(
                "actual installed native camera assertions failed at {stage} ({exception_class}); raw diagnostics omitted"
            )));
        }
        failure_context.exit_code = exited.code().filter(|code| (-999..=999).contains(code));
        return Err(io::Error::other(failure_context));
    }
    if failure_context.automation_receipt_display_invalid {
        return Err(failure("duplicate canonical receipt-display observation"));
    }
    Ok(
        json!({"class":class,"status":"pass","statuses":statuses,"automation_effects":failure_context.automation_effects,"automation_receipt_display":failure_context.automation_receipt_display,"scope":if port.is_some(){"actual packaged host/provider MainActivity consumer"}else{"installed actual-surface component; never stock provider conformance"}}),
    )
}
fn canonical_receipt_display_verified(
    result: &Value,
    development_feedback: bool,
) -> io::Result<bool> {
    let observations = result
        .get("automation_receipt_display")
        .and_then(Value::as_object)
        .ok_or_else(|| failure("canonical receipt-display observation missing"))?;
    if observations.is_empty() && development_feedback {
        return Ok(false);
    }
    if observations.len() != 2
        || ["omp", "codex"].into_iter().any(|provider| {
            observations.get(provider).and_then(Value::as_str) != Some("canonical_recovery_cleared")
        })
    {
        return Err(failure("canonical receipt-display observations incomplete"));
    }
    Ok(true)
}
async fn screenshot(l: &Lease, evidence: &Path, run: &Uuid, scene: &Scene) -> io::Result<Value> {
    let bytes = output(
        adb_command(l, &["exec-out", "screencap", "-p"]).await?,
        10,
        8 * 1024 * 1024,
    )
    .await?;
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(failure("actual guest screenshot is not PNG"));
    }
    let path = evidence.join(format!(
        "ui-{}-{}-{}.png",
        run.0,
        scene.name(),
        Uuid::random().0
    ));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    std::io::Write::write_all(&mut file, &bytes)?;
    file.sync_all()?;
    Ok(
        json!({"scene":scene.name(),"path":path,"capturedAt":pairing::now_ms(),"privacy":"consumer asserts raw pairing/secret/form/URL absent and FLAG_SECURE false before requesting this screenshot"}),
    )
}
pub(super) struct Context<'a> {
    pub(super) binary: &'a Path,
    pub(super) root: &'a Path,
    pub(super) socket: &'a Path,
    pub(super) config: &'a Config,
    pub(super) herdr_socket: &'a Path,
    pub(super) herdr_process: &'a ProcessEvidence,
    pub(super) mcp: Option<&'a NativeMcp>,
    pub(super) serve: &'a mut Companion,
    pub(super) listener: &'a std::net::TcpListener,
    pub(super) endpoint: &'a iroh::Endpoint,
    pub(super) phone: &'a mut Phone,
    pub(super) native: &'a mut [NativeProof],
    pub(super) history_witnesses: &'a [NativeHistoryWitness],
    pub(super) feedback_presentation: &'a [super::FeedbackPresentation],
}

/// Reads the original emulator/package/artifact witnesses only; it does not certify delivery,
/// launch a distributor, select its mode, reset an APK, or create provider resources.
pub(super) async fn background_preflight(args:&clap::ArgMatches)->io::Result<()> {
    super::background_mode_environment(args)?;
    let lease=super::selected_lease(args,"finite background delivery")?;
    check_emulator(&lease).await?;
    for (path,pin) in [("apk","expected-apk-sha256"),("test-apk","expected-test-apk-sha256")] {
        let file=fs::canonicalize(args.get_one::<PathBuf>(path).ok_or_else(||failure("background requires explicit source-qualified APP and TEST APKs"))?)?;
        development_hash(args,pin,&binary_sha256(&file)?)?;
    }
    if super::camera::app_process(&lease).await?.is_some(){return Err(prerequisite("background refuses a prior live Goatr fixture; retain it for Main-reviewed scoped cleanup"));}
    if args.get_one::<String>("background").map(String::as_str)==Some("ntfy") {
        let listed=adb(&lease,&["shell","'pm'","'path'","'io.heckel.ntfy'"]).await?;
        let lines=listed.lines().filter(|line|!line.trim().is_empty()).collect::<Vec<_>>();
        let path=match lines.as_slice(){[line]=>line.trim().strip_prefix("package:").filter(|path|path.starts_with("/data/app/")&&path.ends_with("/base.apk")&&path.len()<=4096&&!path.chars().any(char::is_control)).ok_or_else(||prerequisite("official ntfy has no single installed base APK witness"))?,_=>return Err(prerequisite("qualified official ntfy must already be installed; no helper install or distributor fallback"))};
        let quoted_path=shell_literal(path)?;
        let actual=adb(&lease,&["shell","'sha256sum'",&quoted_path]).await?;
        if actual.split_whitespace().next()!=args.get_one::<String>("expected-ntfy-apk-sha256").map(String::as_str){return Err(prerequisite("installed ntfy bytes do not match the independently qualified official APK pin"));}
    }
    Ok(())
}

/// A finite real-provider consumer in the existing owned-emulator runner. Its only native
/// inputs are the already guarded fresh-original marker/dialog choreography. No fault stop,
/// provider cleanup, forged push, health socket, or mode/distributor fallback is admitted.
pub(super) async fn background(args:&clap::ArgMatches,evidence:&Path,context:Context<'_>,run:&Uuid)->io::Result<()> {
    background_preflight(args).await?;
    let Context{binary,root,socket,config,herdr_socket,herdr_process,mcp,serve,listener:_,endpoint:_,phone,native,history_witnesses:_,feedback_presentation:_}=context;
    if native.len()!=2||!native.iter().any(|p|p.provider==Provider::Omp)||!native.iter().any(|p|p.provider==Provider::Codex){return Err(failure("background requires both fresh normally-created original proofs"));}
    serve.recheck()?;
    for original in native.iter(){original.recheck_processes()?;}
    let lease=super::selected_lease(args,"finite background delivery")?;
    let device=check_emulator(&lease).await?;
    let apk=fs::canonicalize(args.get_one::<PathBuf>("apk").ok_or_else(||failure("background APP APK absent"))?)?;
    let test=fs::canonicalize(args.get_one::<PathBuf>("test-apk").ok_or_else(||failure("background TEST APK absent"))?)?;
    let apk_hash=binary_sha256(&apk)?;let test_hash=binary_sha256(&test)?;
    development_hash(args,"expected-apk-sha256",&apk_hash)?;development_hash(args,"expected-test-apk-sha256",&test_hash)?;
    let mode=args.get_one::<String>("background").ok_or_else(||failure("background mode absent"))?;
    reset_owned_apks(&lease).await?;
    install_owned_apk(&lease,&apk,false).await?;
    install_owned_apk(&lease,&test,true).await?;
    let quoted_app=shell_literal(APP)?;
    adb(&lease,&["shell","'pm'","'grant'",&quoted_app,"'android.permission.POST_NOTIFICATIONS'"]).await?;
    let control=tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,0)).await?;
    let port=control.local_addr()?.port();let mapping=format!("tcp:{port}");
    if adb(&lease,&["reverse","--list"]).await?.lines().any(|line|line.split_whitespace().any(|part|part==mapping)){return Err(failure("background refuses an existing control reverse mapping"));}
    write_json(&evidence.join(format!("background-owner-{}.json",run.0)),&json!({"runId":run,"mode":mode,"device":device,"apk":{"path":apk,"sha256":apk_hash},"testApk":{"path":test,"sha256":test_hash},"ntfy":{"apkSha256":args.get_one::<String>("expected-ntfy-apk-sha256"),"signerSha256":args.get_one::<String>("expected-ntfy-signer-sha256"),"scope":"independently qualified installed package; real registration/probe/ACK remains required"},"native":native.iter().map(NativeProof::evidence).collect::<Vec<_>>(),"reverse":{"local":mapping,"remote":mapping},"resourcesRetained":true,"automaticCleanup":false,"privacy":"original signed instrumentation admission; no keys, endpoints, capability URI, prompt or dialog contents retained"}))?;
    let class=if mode=="ntfy"{"io.github.trevarj.goatr.GoatrConnectionTest#stock_background_ntfy_real_native_both_providers"}else{"io.github.trevarj.goatr.GoatrConnectionTest#stock_background_persistent_real_native_both_providers"};
    let (admission,mut admitted)=watch::channel(None);
    let (retained_epoch,_retained)=watch::channel(None);
    let provenance=InstrumentationEvidence{cleanup_admission:&retained_epoch,evidence,run,apk:&apk,apk_sha256:&apk_hash,test_apk:&test,test_apk_sha256:&test_hash};
    let mut child=Box::pin(instrument_owned(&lease,class,Some(port),false,None,&admission,Some(&provenance)));
    let inputs=NativeInputs{enabled:args.get_flag("automate-native-inputs"),paging_proof:false,config,socket:herdr_socket,herdr:herdr_process,evidence,run,mcp,plan:RefCell::new(None),mcp_dialogs:RefCell::new(HashMap::new())};
    let host=phone.hello.host_id.clone();
    let mut completed=HashSet::new();let mut dialogs=HashSet::new();let mut pending=HashMap::<Uuid,Target>::new();let mut complete=false;
    let result=cancel_on_signal(async {
        adb(&lease,&["reverse","--no-rebind",&mapping,&mapping]).await?;
        loop {
            if admitted.borrow().is_none(){tokio::select!{result=child.as_mut()=>break result,changed=admitted.changed()=>{changed.map_err(|_|failure("background original instrumentation admission closed"))?;}}continue;}
            let handler=async {
                let (mut stream,peer)=control.accept().await?;
                if !peer.ip().is_loopback(){return Ok(());}
                let epoch=(*admitted.borrow()).ok_or_else(||failure("background original app epoch absent"))?;
                let public=epoch.public.ok_or_else(||failure("background original signing identity absent"))?;
                let request=match authenticated_control(&mut stream,&public).await{Ok(request)=>request,Err(_)=>return Ok(())};
                if super::camera::app_process(&lease).await?!=Some(epoch.process){return Err(failure("background original app process changed; no controller effect admitted"));}
                serve.recheck()?;
                let response=match request {
                    Control::LivePair{}=>{
                        let b=binary.to_owned();let r=root.to_owned();let s=socket.to_owned();
                        let mut uri=tokio::task::spawn_blocking(move||terminal_pair(&b,Some(&r),Some(&s))).await.map_err(io::Error::other)??;
                        let value=json!({"uri":uri,"host":host,"runId":run});unsafe{uri.as_bytes_mut().fill(0)};value
                    },
                    Control::LiveInfo{}=>{
                        let inventory=phone.host().await?;
                        let sessions=native.iter().map(|proof|inventory.sessions.iter().find(|row|shared_summary_matches(row,proof)).map(|_|json!({"provider":proof.provider,"target":proof.target,"nativeId":proof.native})).ok_or_else(||failure("background original native source disappeared"))).collect::<io::Result<Vec<_>>>()?;
                        json!({"host":host,"runId":run,"background":mode,"sessions":sessions,"appApkSha256":apk_hash,"testApkSha256":test_hash,"ntfyApkSha256":args.get_one::<String>("expected-ntfy-apk-sha256"),"ntfySignerSha256":args.get_one::<String>("expected-ntfy-signer-sha256")})
                    },
                    Control::Desktop{provider}=>{
                        if !completed.insert(provider){return Err(failure("background native completion cue is once-only; never replay"));}
                        let original=native.iter().find(|p|p.provider==provider).ok_or_else(||failure("background provider was not selected"))?;
                        let marker=format!("goatr-ui-background-{}-{provider:?}",&run.0[..8]);
                        inputs.marker(phone,original,&marker).await?;
                        wait_history(phone,&original.target,&marker,false).await?;
                        original.recheck_processes()?;
                        json!({"marker":marker,"target":original.target,"nativeId":original.native})
                    },
                    Control::TriggerDialog{provider,kind}=>{
                        let expected=if provider==Provider::Omp{PresentationKind::OmpSelect}else{PresentationKind::CodexForm};
                        if kind!=expected||!dialogs.insert(provider){return Err(failure("background native attention cue is once-only and closed to the selected supported form"));}
                        let original=native.iter().find(|p|p.provider==provider).ok_or_else(||failure("background provider was not selected"))?;
                        let request=inputs.trigger_dialog(phone,original,kind,false,None).await?;
                        if pending.insert(request.request_key.clone(),original.target.clone()).is_some(){return Err(failure("background native pending request repeated"));}
                        json!({"requestKey":request.request_key,"kind":kind,"target":original.target})
                    },
                    Control::DialogClosed{target,request_key,receipt}=>{
                        if pending.get(&request_key)!=Some(&target)||receipt.target.as_ref()!=Some(&target)||receipt.method!=ActionMethod::RequestRespond{return Err(failure("background answer is not the original native request"));}
                        let original=native.iter().find(|p|p.target==target).ok_or_else(||failure("background answer changed original Target"))?;
                        if receipt.error.is_some()||receipt.state!=if original.provider==Provider::Omp{ReceiptState::Dispatched}else{ReceiptState::Succeeded}{return Err(failure("background native answer outcome is not confirmed; retained, never replayed"));}
                        let deadline=Instant::now()+Duration::from_secs(15);
                        while phone.open(&target,"background_native_dialog_closed").await?.pending_requests.iter().any(|r|r.request_key==request_key){if Instant::now()>deadline{return Err(failure("background original native request did not close"));}tokio::time::sleep(Duration::from_millis(100)).await;}
                        inputs.finish_dialog(phone,original,&request_key,false).await?;
                        pending.remove(&request_key);json!({"verified":true})
                    },
                    Control::Complete{}=>{
                        if completed.len()!=2||dialogs.len()!=2||!pending.is_empty(){return Err(failure("background cannot pass an omitted selected provider or unresolved native request"));}
                        for original in native.iter(){original.recheck_processes()?;}
                        complete=true;json!({"complete":true})
                    },
                    _=>return Err(failure("background rejects unrelated UI, replay, fault-stop, cleanup or capture controls")),
                };
                let mut encoded=serde_json::to_vec(&response).map_err(io::Error::other)?;
                if encoded.len()>65536{return Err(failure("background control response bound"));}
                encoded.push(b'\n');tokio::time::timeout(Duration::from_secs(5),stream.write_all(&encoded)).await.map_err(|_|failure("background control reply deadline"))??;encoded.fill(0);
                Ok::<(),io::Error>(())
            };
            match poll_instrumentation_control(child.as_mut(),handler).await {
                InstrumentationControl::Instrumentation(result)=>break result,
                InstrumentationControl::Handler(result)=>result?,
            }
        }
    }).await;
    drop(child);
    let result=result?;
    if !complete{return Err(failure("background actual instrumentation omitted the full two-provider completion"));}
    write_json(&evidence.join(format!("background-{}.json",run.0)),&json!({"gate":"background","status":"pass","mode":mode,"runId":run,"android":result,"providers":native.iter().map(NativeProof::evidence).collect::<Vec<_>>(),"originalApp":admitted.borrow().map(|epoch|json!({"pid":epoch.process.0,"startTicks":epoch.process.1.to_string()})),"resourcesRetained":true,"automaticCleanup":false,"notificationEntry":"system_ui_native_row_click","routerComponents":"busy admission, newer navigation and foreground-loss retirement after a genuine OS tap","scope":"real registration/probe/ACK for ntfy and real native attention/completion with actual SystemUI notification-row clicks and rendered exact-original conversations for both providers; activity-cold and warm, not a process-kill/Wi-Fi/cellular claim"}))?;
    println!("{}",json!({"gate":"background","mode":mode,"status":"pass","runId":run,"resourcesRetained":true}));
    Ok(())
}

pub(super) async fn run(
    args: &clap::ArgMatches,
    evidence: &Path,
    context: Context<'_>,
    run: &Uuid,
) -> io::Result<()> {
    let Context {
        binary,
        root,
        socket,
        config,
        herdr_socket,
        herdr_process,
        mcp,
        serve,
        listener,
        endpoint,
        phone,
        native,
        history_witnesses,
        feedback_presentation: _,
    } = context;
    let inputs = NativeInputs {
        enabled: args.get_flag("automate-native-inputs"),
        paging_proof: false,
        config,
        socket: herdr_socket,
        herdr: herdr_process,
        evidence,
        run,
        mcp,
        plan: RefCell::new(None),
        mcp_dialogs: RefCell::new(HashMap::new()),
    };
    let l = super::selected_lease(args, "UI runner")?;
    let device = check_emulator(&l).await?;
    if super::camera::app_process(&l).await?.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "UI runner refuses a prior/unowned live debug APK process before installation",
        ));
    }
    let apk = fs::canonicalize(
        args.get_one::<PathBuf>("apk")
            .cloned()
            .unwrap_or_else(|| PathBuf::from("app/build/outputs/apk/debug/app-debug.apk")),
    )?;
    let test_apk = fs::canonicalize(args.get_one::<PathBuf>("test-apk").cloned().unwrap_or_else(
        || PathBuf::from("app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"),
    ))?;
    let apk_hash = binary_sha256(&apk)?;
    let test_hash = binary_sha256(&test_apk)?;
    reset_owned_apks(&l).await?;
    install_owned_apk(&l, &apk, false).await?;
    install_owned_apk(&l, &test_apk, true).await?;
    // Native tests prove actual JNI APIs; native-inventory example separately proves all ten
    // exact ABI/ELF/ZIP/transformed producer/Maven/signing/source-license identities.
    let mut checks = vec![
        instrument(
            &l,
            "io.github.trevarj.goatr.NativeLoadTest,io.github.trevarj.goatr.NativeInventoryTest",
            None,
            false,
            None,
        )
        .await?,
    ];
    let mut paging_regression = instrument(
        &l,
        "io.github.trevarj.goatr.SessionStoreTest#olderWindowRetainsRequestedRowsAcrossLiveUpdatesAndResetsOnlyOnSnapshot",
        None,
        false,
        None,
    )
    .await?;
    paging_regression["scope"] =
        json!("installed reducer regression only; never stock provider or UI acceptance");
    checks.push(paging_regression);
    checks.push(instrument(&l, "io.github.trevarj.goatr.ProviderFormsTest#actualAospImeVisibleKeysEnterBoundedNegativeDecimalAndInteger", None, false, None).await?);
    // Permission changes affect only the owned debug package, before instrumentation starts.
    adb(
        &l,
        &["shell", "pm", "revoke", APP, "android.permission.CAMERA"],
    )
    .await?;
    adb(
        &l,
        &[
            "shell",
            "pm",
            "clear-permission-flags",
            APP,
            "android.permission.CAMERA",
            "user-set",
            "user-fixed",
        ],
    )
    .await?;
    checks.push(instrument(&l, "io.github.trevarj.goatr.PairingUiTest#deniedCameraLeavesActualPasteAndDisposedQueuedCallbacksCannotAccept", None, false, None).await?);
    adb(
        &l,
        &["shell", "pm", "grant", APP, "android.permission.CAMERA"],
    )
    .await?;
    checks.push(super::camera::exercise(&l).await?);
    let control = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = control.local_addr()?.port();
    let mapping = format!("tcp:{port}");
    let list = adb(&l, &["reverse", "--list"]).await?;
    if list
        .lines()
        .any(|line| line.split_whitespace().any(|s| s == mapping))
    {
        return Err(failure(
            "owned control reverse port already mapped; never adopt/replace",
        ));
    }
    write_json(
        &evidence.join(format!("ui-owner-{}.json", run.0)),
        &json!({"runId":run,"device":device,"apk":apk,"apkSha256":apk_hash,"testApk":test_apk,"testApkSha256":test_hash,"controlPort":port,"reverse":{"local":mapping,"remote":mapping},"privacy":"per-invocation public P-256 key + original PID/start admitted only from owned instrumentation stream; fresh challenge signatures over exact bounded requests; private key/live URI/originals memory-only, never argv/env/log/clipboard/evidence"}),
    )?;
    let baseline_focus = focus(&surface(herdr_socket).await?.1);
    let anchor = surface(herdr_socket)
        .await?
        .1
        .focused_pane_id
        .ok_or_else(|| failure("native focused pane absent for UI geometry"))?;
    let baseline_geometry = geometry(herdr_socket, &anchor, GeometryPhase::UiBefore)
        .await
        .map_err(GeometryFailure::into_io)?;
    let host = phone.hello.host_id.clone();
    let class = "io.github.trevarj.goatr.GoatrConnectionTest#stock_packaged_host_actual_main_activity_both_selected_providers";
    let (admission, mut admitted) = watch::channel(None);
    let mut child = Box::pin(instrument_owned(
        &l,
        class,
        Some(port),
        false,
        None,
        &admission,
        None,
    ));
    let mut prior_public = None;
    let mut complete = false;
    let mut scenes = Vec::new();
    let mut observed = Vec::new();
    let mut omp_select: Option<Uuid> = None;
    let mut triggered = HashMap::<Uuid, (Target, PresentationKind)>::new();
    let mut death: Option<Value> = None;
    let mut death_count = 0usize;
    let result = cancel_on_signal(async {
    adb(&l, &["reverse", "--no-rebind", &mapping, &mapping]).await?;
    loop {
        if admitted.borrow().is_none() {
            tokio::select! {
                result = &mut child => break result,
                changed = admitted.changed() => {
                    changed.map_err(|_| failure("owned instrumentation admission stream closed"))?;
                }
            }
            continue;
        }
  tokio::select!{
   result=&mut child=>{break result;},
   // Cancellation surrounds this entire operation, including every selected handler.
   incoming=control.accept()=>{
    let (mut stream,peer)=incoming?;
    if !peer.ip().is_loopback(){continue;}
    let epoch=(*admitted.borrow()).ok_or_else(||failure("stock-control original epoch unavailable"))?;
    let public=epoch.public.ok_or_else(||failure("stock-control admitted signing identity absent"))?;
    if prior_public==Some(public){return Err(failure("recovery reused prior instrumentation signing identity"));}
    let request=match authenticated_control(&mut stream,&public).await {Ok(request)=>request,Err(_)=>continue};
    if super::camera::app_process(&l).await?!=Some(epoch.process){return Err(failure("authenticated stock-control original app epoch changed; no effect admitted"));}
    let response=match request {
     Control::LivePair{}=>{let b=binary.to_owned();let r=root.to_owned();let s=socket.to_owned();let mut uri=tokio::task::spawn_blocking(move||terminal_pair(&b,Some(&r),Some(&s))).await.map_err(io::Error::other)??;let value=json!({"uri":uri,"host":host,"runId":run});unsafe{uri.as_bytes_mut().fill(0)};value},
     Control::LiveInfo{}=>{let inventory=phone.host().await?;let own=native.first().ok_or_else(||failure("no selected actual UI provider"))?;let actual=inventory.sessions.iter().find(|s|s.session_id==own.target.session_id).ok_or_else(||failure("actual UI server missing"))?;let row=inventory.servers.iter().find(|s|s.server_id==actual.server_id).ok_or_else(||failure("actual named UI server missing"))?;json!({"host":host,"runId":run,"deferRetentionCrossing":args.get_flag("defer-retention-crossing"),"serverId":actual.server_id,"serverName":row.name,"projectId":config.projects[0].id,"projectLabel":config.projects[0].label,"sessions":native.iter().map(|p|json!({"provider":p.provider,"target":p.target,"nativeId":p.native,"pane":p.pane,"terminal":p.terminal,"workspace":p.workspace})).collect::<Vec<_>>()})},
     Control::Observe{target,marker,receipt}=>{if marker.len()>128||!marker.starts_with("goatr-ui-")||receipt.method!=ActionMethod::PromptSubmit||receipt.target.as_ref()!=Some(&target)||!matches!(receipt.state,ReceiptState::Dispatched|ReceiptState::Succeeded){return Err(failure("actual UI prompt correlation/receipt mismatch"));}let p=native.iter().find(|p|p.target==target).ok_or_else(||failure("UI changed/foreign Target"))?;p.recheck_processes()?;let state=wait_history(phone,&target,&marker,true).await?;observed.push(json!({"scenario":"ui-native-prompt-history-tool","target":target,"action":receipt.action_id,"receipt":receipt,"nativeProof":p.evidence(),"recordIds":state.history.iter().map(|r|&r.id).collect::<Vec<_>>() }));json!({"verified":true})},
     Control::Older{target}=>{
      if args.get_flag("defer-retention-crossing"){return Err(failure("exhaustive older witness explicitly deferred; no witness fabricated"));}
      let p=native.iter().find(|p|p.target==target).ok_or_else(||failure("UI older history Target changed"))?;
      let witness=history_witnesses.iter().find(|w|w.original.session_id==target.session_id&&w.provider==p.provider&&w.native==p.native&&w.owner==p.owner).ok_or_else(||failure("UI older history original native witness absent"))?;
      if witness.older.is_empty()||witness.older.len()>16||witness.retained.len()>goatr::state::HISTORY_RECORDS||witness.qualified_later_completed_lower_bound<1000||!witness.older.contains(&witness.qualified){return Err(failure("UI older history original boundary invalid"));}
      inputs.current(phone,p).await?;
      let open=phone.open(&target,"ui_older_original").await?;
      if open.state.summary.native_id!=p.native||super::target(&open.state.summary)!=target{return Err(failure("UI older history native changed"));}
      let retained: HashSet<_> = open.history.iter().map(|row| row.id.clone()).collect(); let mut seen=HashSet::new();let mut cursors=HashSet::new();let mut cursor: Option<Id>=None;let mut view_id: Option<Uuid>=None;let mut older=None;
      for _ in 0..64{
       if let Some(id)=&cursor && !cursors.insert(id.clone()){return Err(failure("UI older native cursor repeated"));}
       let ResultBody::SessionHistory(page)=phone.rpc(RequestBody::SessionHistory(SessionHistoryParams{target:target.clone(),view_id:view_id.clone(),cursor:cursor.clone(),limit:Some(Uint(100))}),"ui_older_source").await? else{return Err(failure("UI older history page absent"));};
       if view_id.as_ref().is_some_and(|v|v!=&page.view_id){return Err(failure("UI native view changed"));}
       view_id=Some(page.view_id.clone());
       for record in page.records{
        if !record.complete{return Err(failure("UI older source incomplete record"));}
        if seen.insert(record.id.clone()) && record.id==witness.qualified && !retained.contains(&record.id) && record.source==if p.provider==Provider::Omp{RecordSource::Persisted}else{RecordSource::NativePage}{older=Some(record.id);}
       }
       if older.is_some(){break;}cursor=page.next_cursor;if cursor.is_none(){break;}
      }
      let id=older.ok_or_else(||prerequisite("UI independently witnessed native ID unavailable within the original 64-advance traversal budget"))?;
      inputs.current(phone,p).await?;
      observed.push(json!({"scenario":"ui-original-older-witness-revalidated","target":target,"originalTarget":witness.original,"id":id,"qualifiedLaterCompletedLowerBound":witness.qualified_later_completed_lower_bound,"latestSnapshotIds":retained,"localWindowLimit":goatr::state::HISTORY_RECORDS,"maxCursorAdvances":64,"viewExpirySeconds":900,"exhaustiveRetentionCrossing":"proved","nativeProof":p.evidence()}));
      json!({"target":target,"id":id,"qualifiedLaterCompletedLowerBound":witness.qualified_later_completed_lower_bound,"source":if p.provider==Provider::Omp{"persisted"}else{"native_page"}})
     },
     Control::Desktop{provider}=>{let p=native.iter().find(|p|p.provider==provider).ok_or_else(||failure("requested UI backend not selected"))?;let marker=format!("goatr-ui-desktop-{}-{:?}",&run.0[..8],provider);inputs.marker(phone,p,&marker).await?;wait_history(phone,&p.target,&marker,false).await?;json!({"marker":marker})},
     Control::TriggerDialog{provider,kind}=>{
      let p=native.iter().find(|p|p.provider==provider).ok_or_else(||failure("requested UI dialog backend not selected"))?;
      let request=inputs.trigger_dialog(phone,p,kind,true,omp_select.as_ref()).await?;
      if kind==PresentationKind::OmpSelect {omp_select=Some(request.request_key.clone());}
      if triggered.insert(request.request_key.clone(),(p.target.clone(),kind)).is_some(){return Err(failure("UI native trigger repeated an already-owned pending request"));}
      json!({"requestKey":request.request_key,"kind":kind})
     },
     Control::DialogClosed{target,request_key,receipt}=>{
      if receipt.target.as_ref()!=Some(&target)||receipt.method!=ActionMethod::RequestRespond{return Err(failure("actual UI dialog receipt target/method mismatch"));}
      let p=native.iter().find(|p|p.target==target).ok_or_else(||failure("UI dialog Target changed"))?;
      if triggered.remove(&request_key).is_none_or(|(original,_)|original!=target){return Err(failure("UI dialog closure was not the original observed native request"));}
      if receipt.error.is_some()||receipt.state!=if p.provider==Provider::Omp{ReceiptState::Dispatched}else{ReceiptState::Succeeded}{return Err(failure("actual UI dialog receipt did not reach native accepted dispatch"));}
      p.recheck_processes()?;let deadline=Instant::now()+Duration::from_secs(15);
      while phone.open(&target,"ui_dialog_closed").await?.pending_requests.iter().any(|r|r.request_key==request_key){if Instant::now()>deadline{return Err(failure("actual native pending card never closed after visible response"));}tokio::time::sleep(Duration::from_millis(100)).await;}
      inputs.finish_dialog(phone,p,&request_key,false).await?;
      observed.push(json!({"scenario":"ui-actual-stock-dialog","target":target,"requestKey":request_key,"receipt":receipt}));
      json!({"verified":true})
     },
     Control::Gap{device_id}=>{if goatr::admin::call(socket,"devices.list",json!({})).await?["devices"].as_array().is_none_or(|d|!d.iter().any(|p|p["deviceId"].as_str()==Some(&device_id.0))){return Err(failure("UI gap peer not actually paired"));}serve.stop().await?;for p in native.iter(){p.recheck_processes()?;}json!({"stopped":true})},
     Control::Restart{}=>{*serve=start_serve(binary,&root.join("config.toml"),root,socket,Some(listener))?;healthy(serve,socket).await?;phone.connection.close(0u32.into(),b"UI gap/restart");*phone=connect(endpoint,&host,None).await?;let deadline=Instant::now()+Duration::from_secs(30);loop{let h=phone.host().await?;if native.iter().all(|p|h.sessions.iter().any(|s|s.session_id==p.target.session_id&&s.native_id==p.native&&s.connectivity==SessionConnectivity::Ready)){for p in native.iter_mut(){let s=h.sessions.iter().find(|s|s.session_id==p.target.session_id).ok_or_else(||failure("UI restart native missing"))?;let fresh=proof(config,herdr_socket,s,None).await?;if p.owner!=fresh.owner||p.tui!=fresh.tui||p.relay!=fresh.relay||p.native!=fresh.native{return Err(failure("UI restart replaced native desktop epochs"));}*p=fresh;}break;}
     if Instant::now()>deadline{return Err(failure("UI actual restart reconciliation unavailable"));}tokio::time::sleep(Duration::from_millis(100)).await;}json!({"sessions":native.iter().map(|p|json!({"provider":p.provider,"target":p.target,"nativeId":p.native})).collect::<Vec<_>>()})},
     Control::Revoke{device_id}=>{goatr::admin::call(socket,"device.revoke",json!({"deviceId":device_id})).await?;for p in native.iter(){p.recheck_processes()?;}json!({"revoked":true})},
     Control::Capture{scene}=>{let image=screenshot(&l,evidence,run,&scene).await?;scenes.push(image);json!({"captured":true})},
     Control::VerifyCreate{receipt}=>{if receipt.method!=ActionMethod::SessionCreate{return Err(failure("UI creation receipt method mismatch"));}let creation=receipt.creation.as_ref().ok_or_else(||failure("UI partial creation resources not retained"))?;let (_,actual)=surface(herdr_socket).await?;if focus(&actual)!=baseline_focus{return Err(failure("actual UI creation changed desktop focus"));}
     if let Some(pane)=&creation.resources.pane_id{actual.pane(pane)?;}
     if receipt.state==ReceiptState::Succeeded{let h=phone.host().await?;let s=h.sessions.iter().find(|s|creation.resources.session_id.as_ref()==Some(&s.session_id)).ok_or_else(||failure("UI created ready target not published"))?;let exact=proof(config,herdr_socket,s,None).await?;observed.push(json!({"scenario":"ui-no-focus-create","receipt":receipt,"proof":exact.evidence()}));}else if receipt.state==ReceiptState::Uncertain&&creation.resources.workspace_id.is_some(){observed.push(json!({"scenario":"ui-actual-partial-uncertain-no-replay","receipt":receipt,"nativeWorkRetained":true}));}else{return Err(failure("UI creation did not expose required Ready or genuine partial/uncertain outcome"));}json!({"verified":true})},
     Control::FaultCreate{receipt}=>{let creation=receipt.creation.as_ref().ok_or_else(||failure("actual UI partial creation source absent"))?;if receipt.method!=ActionMethod::SessionCreate||receipt.state!=ReceiptState::Dispatching||creation.resources.workspace_id.is_none(){return Err(failure("fault admission requires observed actual incomplete creation with retained workspace"));}let (_,before)=surface(herdr_socket).await?;if focus(&before)!=baseline_focus{return Err(failure("UI creation took desktop focus"));}serve.recheck()?;if unsafe{libc::kill(serve.process.pid as i32,libc::SIGKILL)}!=0{return Err(io::Error::last_os_error());}serve.child.wait()?;for p in native.iter(){p.recheck_processes()?;}observed.push(json!({"scenario":"actual-ui-create-crash-after-retained-resource","beforeReceipt":receipt,"focusUnchanged":true}));json!({"crashed":true})},
     Control::ProcessDeath{device_id,action_id,target,expires_at}=>{
      if death.is_some()||expires_at.0<=pairing::now_ms()||expires_at.0>pairing::now_ms()+7*24*60*60*1000{return Err(failure("actual APK process-death recovery lease/deadline mismatch"));}
      let p=native.iter().find(|p|p.target==target).ok_or_else(||failure("process-death Target not the actual selected native session"))?;
      if goatr::admin::call(socket,"devices.list",json!({})).await?["devices"].as_array().is_none_or(|rows|!rows.iter().any(|r|r["deviceId"].as_str()==Some(&device_id.0))){return Err(failure("process-death Android peer is not actually paired"));}
      let marker=format!("goatr-ui-death-{}",&run.0[..8]);let state=wait_history(phone,&target,&marker,false).await?;
      death_count=state.history.iter().filter(|r|r.role==Role::User&&r.text.as_ref().is_some_and(|c|matches!(c,Content::Text(t) if t.text.0.contains(&marker)))).count();
      if death_count!=1{return Err(failure("actual pre-death native user prompt must occur exactly once"));}
      let (pid,ticks)=epoch.process;
      let guest_boot=adb(&l,&["shell","cat","/proc/sys/kernel/random/boot_id"]).await?;
      if super::camera::app_process(&l).await?!=Some(epoch.process){return Err(failure("stock OS death refuses replacement/unproved original APK process"));}
      if !super::camera::stop_app(&l,Some(epoch.process)).await?{return Err(failure("original app exited before intentional owned OS death admission"));}
      death=Some(json!({"ownedProcessDied":true,"host":host,"deviceId":device_id,"actionId":action_id,"target":target,"expiresAt":expires_at,"guestPid":pid,"guestStart":ticks.to_string(),"guestBoot":guest_boot.trim(),"nativeProof":p.evidence()}));
      p.recheck_processes()?;
      let interrupted=tokio::time::timeout(Duration::from_secs(15),&mut child).await.map_err(|_|failure("killed APK instrumentation did not terminate"))?;
      if interrupted.is_ok(){return Err(failure("first consumer survived forced OS death; no fabricated process-death pass"));}
      checks.push(json!({"class":class,"status":"pass","scope":"all prior real UI phases reached observed owned APK process-death boundary; interrupted instrumentation is expected ONLY after proved OS death","lease":death}));
      prior_public=epoch.public;
      admission.send_replace(None);
      child=Box::pin(instrument_owned(&l,"io.github.trevarj.goatr.GoatrConnectionTest#stock_process_death_restores_original_target_receipt_without_replay_then_forgets",Some(port),false,None,&admission,None));
      continue;
     },
     Control::RecoveryInfo{}=>death.clone().ok_or_else(||failure("no actually observed prior owned APK process death; never adopt existing keys"))?,
     Control::VerifyNoReplay{action_id,target,receipt}=>{
      let lease=death.as_ref().ok_or_else(||failure("process-death lease absent"))?;
      if lease["actionId"].as_str()!=Some(&action_id.0)||serde_json::to_value(&target).map_err(io::Error::other)?!=lease["target"]||receipt.action_id!=action_id||receipt.target.as_ref()!=Some(&target){return Err(failure("actual recovered original UUID/Target/receipt mismatch"));}
      let marker=format!("goatr-ui-death-{}",&run.0[..8]);let state=phone.open(&target,"ui_recovery_count").await?;
      let count=state.history.iter().filter(|r|r.role==Role::User&&r.text.as_ref().is_some_and(|c|matches!(c,Content::Text(t) if t.text.0.contains(&marker)))).count();
      if count!=death_count{return Err(failure("actual OS restore replayed or lost the original native prompt"));}
      for p in native.iter(){p.recheck_processes()?;}
      observed.push(json!({"scenario":"actual-APK-process-death-original-no-replay","lease":lease,"receipt":receipt,"nativeUserCountBefore":death_count,"nativeUserCountAfter":count}));json!({"verified":true})
     },
     Control::PhysicalReport{..}|Control::FeedbackPaired{..}|Control::FeedbackReady{..}|Control::DevelopmentReady{..}=>return Err(failure("post-acceptance/development/physical control is unavailable during acceptance")),
     Control::Complete{}=>{if geometry(herdr_socket,&anchor,GeometryPhase::UiAfter).await.map_err(GeometryFailure::into_io)?!=baseline_geometry||focus(&surface(herdr_socket).await?.1)!=baseline_focus{return Err(failure("actual phone flow changed desktop focus/geometry"));}complete=true;json!({"complete":true})}
    };
    let mut encoded=serde_json::to_vec(&response).map_err(io::Error::other)?;if encoded.len()>65536{return Err(failure("stock control response bound"));}encoded.push(b'\n');tokio::time::timeout(Duration::from_secs(5),stream.write_all(&encoded)).await.map_err(|_|failure("stock control response deadline"))??;encoded.fill(0);
   }
  }
 }}).await;
    drop(child);
    let epoch = *admitted.borrow();
    let app_cleanup = super::camera::stop_app(&l, epoch.map(|e| e.process)).await;
    let serve_cleanup = if result.is_err() {
        serve.stop().await
    } else {
        Ok(())
    };
    // Remove only this exact still-matching mapping. Never remove all reverse mappings.
    if emulator_matches(&l) {
        let list = adb(&l, &["reverse", "--list"]).await?;
        if list.lines().any(|s| {
            let v = s.split_whitespace().collect::<Vec<_>>();
            v.len() == 3 && v[1] == mapping && v[2] == mapping
        }) {
            adb(&l, &["reverse", "--remove", &mapping]).await?;
        } else if list
            .lines()
            .any(|line| line.split_whitespace().any(|s| s == mapping))
        {
            return Err(failure("owned reverse mapping changed; cleanup refused"));
        }
    }
    app_cleanup?;
    serve_cleanup?;
    checks.push(result?);
    if !complete {
        return Err(failure(
            "installed actual stock UI scenario did not complete every selected backend",
        ));
    }
    write_json(
        &evidence.join(format!("ui-stock-{}.json", run.0)),
        &json!({"gate":"actual-stock-ui","status":checkpoint_status(args),"exhaustivePaging":exhaustive_paging(args),"runId":run,"device":device,"apkSha256":apk_hash,"testApkSha256":test_hash,"checks":checks,"observed":observed,"screenshots":scenes,"secretOriginals":"not persisted","cameraScope":"actual CameraX/ZXing synthetic never-issued preview/cancel; not live camera authorization or physical hardware","componentScope":"real installed IME/native/JNI tests retain honest component labels"}),
    )?;
    Ok(())
}

/// Bounded installed-app consumer for the fresh development continuity endpoint.
/// It deliberately leaves the paired debug app and companion available for the owner.
fn development_hash(args: &clap::ArgMatches, name: &str, actual: &str) -> io::Result<()> {
    let expected = args
        .get_one::<String>(name)
        .ok_or_else(|| failure("development APK hash is required"))?;
    if expected.len() != 64
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
        || expected.to_ascii_lowercase() != actual
    {
        return Err(failure(
            "development APK differs from its explicit frozen SHA-256",
        ));
    }
    Ok(())
}
async fn development_desktop_attach(
    config: &Config,
    root: &Path,
    herdr_socket: &Path,
    herdr_process: &ProcessEvidence,
    native: &[NativeProof],
    desktop_session: &str,
    desktop_config_home: &Path,
) -> io::Result<Value> {
    let (attach_server, _) = surface(herdr_socket).await?;
    let attach_identity = json!(&attach_server);
    if herdr::process_evidence(herdr_process.pid)? != *herdr_process
        || native
            .iter()
            .any(|proof| proof.objects.get("herdr") != Some(&attach_identity))
    {
        return Err(failure(
            "development desktop attach server differs from the original Herdr process",
        ));
    }
    let desktop_attach = json!({
        "executable":config.executables.herdr,
        "argv":["session","attach",desktop_session],
        "environment":{
            "HERDR_STARTUP_CWD":config.projects[0].path,
            "XDG_CONFIG_HOME":desktop_config_home,
            "XDG_STATE_HOME":root.join("state"),
        },
        "server":attach_server,
        "socket":herdr_socket,
        "verification":"the retained exact server/socket was rechecked before publishing this attach invocation",
    });
    Ok(desktop_attach)
}
pub(super) async fn development(
    args: &clap::ArgMatches,
    evidence: &Path,
    context: Context<'_>,
    run: &Uuid,
    desktop_session: &str,
    desktop_config_home: &Path,
) -> io::Result<()> {
    let Context {
        binary,
        root,
        socket,
        config,
        herdr_socket,
        herdr_process,
        mcp: _,
        serve,
        listener: _,
        endpoint: _,
        phone,
        native,
        history_witnesses: _,
        feedback_presentation,
    } = context;
    let inputs = NativeInputs {
        enabled: true,
        paging_proof: false,
        config,
        socket: herdr_socket,
        herdr: herdr_process,
        evidence,
        run,
        mcp: None,
        plan: RefCell::new(None),
        mcp_dialogs: RefCell::new(HashMap::new()),
    };
    let l = super::selected_lease(args, "development Android consumer")?;
    let device = check_emulator(&l).await?;
    if !args.get_flag("development-owner-feedback")
        && super::camera::app_process(&l).await?.is_some()
    {
        return Err(failure(
            "development Android consumer refuses a prior debug app process",
        ));
    }
    let apk = fs::canonicalize(
        args.get_one::<PathBuf>("apk")
            .ok_or_else(|| failure("development Android consumer requires --apk"))?,
    )?;
    let test_apk = fs::canonicalize(
        args.get_one::<PathBuf>("test-apk")
            .ok_or_else(|| failure("development Android consumer requires --test-apk"))?,
    )?;
    let apk_hash = binary_sha256(&apk)?;
    let test_apk_hash = binary_sha256(&test_apk)?;
    development_hash(args, "expected-apk-sha256", &apk_hash)?;
    development_hash(args, "expected-test-apk-sha256", &test_apk_hash)?;
    if args.get_flag("development-owner-feedback") {
        let desktop_attach = development_desktop_attach(
            config,
            root,
            herdr_socket,
            herdr_process,
            native,
            desktop_session,
            desktop_config_home,
        )
        .await?;
        let source_objects = [root, socket, herdr_socket]
            .into_iter()
            .map(|path| Ok((path, identity(path)?)))
            .collect::<io::Result<Vec<_>>>()?;
        let recheck = || -> io::Result<()> {
            serve.recheck()?;
            if herdr::process_evidence(herdr_process.pid)? != *herdr_process {
                return Err(failure(
                    "owner feedback Herdr process changed; no replacement",
                ));
            }
            super::dev_continuity::zero_native_caps(herdr_process)?;
            for (path, original) in &source_objects {
                if identity(path)? != *original {
                    return Err(failure("owner feedback source object identity changed"));
                }
            }
            for proof in native.iter() {
                proof.recheck_processes()?;
                for process in std::iter::once(&proof.owner)
                    .chain(proof.tui.iter())
                    .chain(proof.relay.iter())
                {
                    super::dev_continuity::zero_native_caps(process)?;
                }
            }
            Ok(())
        };
        recheck()?;
        let mut known_failures = vec![
            "Android automated timeout",
            "older paging unproven",
            "receipt details reachability",
        ];
        if feedback_presentation.iter().any(|observation| {
            observation.focus_changed || observation.geometry_changed == Some(true)
        }) {
            known_failures
                .push("observed native creation presentation change deferred for owner feedback");
        }
        let presentation = feedback_presentation
            .iter()
            .map(|observation| {
                json!({
                    "provider":observation.provider,
                    "focusChanged":observation.focus_changed,
                    "geometryChanged":observation.geometry_changed,
                    "noFocusAcceptance":false,
                })
            })
            .collect::<Vec<_>>();
        let pair = json!({
            "executable":binary,"argv":["pair","--admin-socket",socket],
            "environment":{"XDG_CONFIG_HOME":socket.parent().ok_or_else(||failure("owned admin parent missing"))?.join("config"),"XDG_STATE_HOME":root.join("state")},
            "instruction":"Run in a local terminal to display a fresh invitation; no token was generated by this handoff",
        });
        let record = evidence.join(format!("development-owner-feedback-{}.json", run.0));
        write_json(
            &record,
            &json!({
                "gate":"development-owner-feedback","status":"ready_for_owner","runId":run,
                "root":root,"adminSocket":socket,"companion":serve.process,"herdr":herdr_process,
                "native":native.iter().map(NativeProof::evidence).collect::<Vec<_>>(),
                "package":binary,"packageSha256":binary_sha256(binary)?,"companionLockSha256":transport::lock_sha256(),
                "apk":apk,"apkSha256":apk_hash,"testApk":test_apk,"testApkSha256":test_apk_hash,
                "ownedEmulator":device,"desktopAttach":desktop_attach,"pair":pair,
                "presentation":presentation,"knownFailures":known_failures,
                "humanPhone":"pending","androidAcceptance":"not claimed","apkInstalledByThisMode":false,
                "scope":"fresh original native sessions and companion retained for explicit owner feedback; no automated Android result",
            }),
        )?;
        println!(
            "{}",
            json!({"gate":"development-owner-feedback","status":"ready_for_owner","runId":run,"evidence":record})
        );
        let continuation: io::Result<()> = async {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            loop {
                tokio::select! {
                    result = tokio::signal::ctrl_c() => { result?; break Ok(()); },
                    _ = terminate.recv() => break Ok(()),
                    _ = tokio::time::sleep(Duration::from_secs(2)) => { recheck()?; },
                }
            }
        }
        .await;
        let cleanup = serve.stop().await;
        write_json(
            &evidence.join(format!("development-owner-feedback-outcome-{}.json", run.0)),
            &json!({
                "gate":"development-owner-feedback","runId":run,"status":if continuation.is_ok() {"stopped"} else {"failed"},
                "serveStopped":cleanup.is_ok(),"nativeWorkRetained":true,"androidAcceptance":"not claimed",
            }),
        )?;
        continuation?;
        return cleanup;
    }
    reset_owned_apks(&l).await?;
    install_owned_apk(&l, &apk, false).await?;
    install_owned_apk(&l, &test_apk, true).await?;
    let control = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = control.local_addr()?.port();
    let mapping = format!("tcp:{port}");
    if adb(&l, &["reverse", "--list"])
        .await?
        .lines()
        .any(|line| line.split_whitespace().any(|value| value == mapping))
    {
        return Err(failure(
            "development Android control reverse mapping already exists",
        ));
    }
    let host = phone.hello.host_id.clone();
    write_json(
        &evidence.join(format!("development-android-owner-{}.json", run.0)),
        &json!({
            "runId":run,
            "device":device,
            "apkSha256":apk_hash,
            "testApkSha256":test_apk_hash,
            "companion":serve.process,
            "native":native.iter().map(NativeProof::evidence).collect::<Vec<_>>(),
            "scope":"actual owned debug APK pairs to the fresh continuity companion; no final Android acceptance claim",
        }),
    )?;
    let android_only = args.get_flag("development-android-only");
    let class = if android_only {
        "io.github.trevarj.goatr.GoatrConnectionTest#development_feedback_android_to_desktop"
    } else {
        "io.github.trevarj.goatr.GoatrConnectionTest#development_fresh_sessions_continue_from_android_to_desktop"
    };
    let (admission, mut admitted) = watch::channel(None);
    let (cleanup_admission, cleanup_admitted) = watch::channel(None);
    let instrumentation_evidence = InstrumentationEvidence {
        cleanup_admission: &cleanup_admission,
        evidence,
        run,
        apk: &apk,
        apk_sha256: &apk_hash,
        test_apk: &test_apk,
        test_apk_sha256: &test_apk_hash,
    };
    let mut child = Box::pin(instrument_owned(
        &l,
        class,
        Some(port),
        false,
        None,
        &admission,
        Some(&instrumentation_evidence),
    ));
    let mut complete = false;
    let mut paired_device = None;
    let mut history_observations = None;
    let result = cancel_on_signal(async {
        adb(&l, &["reverse", "--no-rebind", &mapping, &mapping]).await?;
        loop {
            if admitted.borrow().is_none() {
                tokio::select! {
                    result = &mut child => break result,
                    changed = admitted.changed() => changed.map_err(|_| failure("development Android admission stream closed"))?,
                }
                continue;
            }
            tokio::select! {
                result = &mut child => break result,
                incoming = control.accept() => {
                    let handler = async {
                    let (mut stream, peer) = incoming?;
                    if !peer.ip().is_loopback() { return Ok::<(), io::Error>(()); }
                    let epoch = (*admitted.borrow()).ok_or_else(|| failure("development Android original epoch unavailable"))?;
                    let public = epoch.public.ok_or_else(|| failure("development Android signing identity unavailable"))?;
                    let request = match authenticated_control(&mut stream, &public).await { Ok(request) => request, Err(_) => return Ok::<(), io::Error>(()) };
                    if super::camera::app_process(&l).await? != Some(epoch.process) {
                        return Err(failure("development Android app epoch changed before control effect"));
                    }
                    let response = match request {
                        Control::LivePair {} => {
                            let binary = binary.to_owned();
                            let root = root.to_owned();
                            let socket = socket.to_owned();
                            let mut uri = tokio::task::spawn_blocking(move || terminal_pair(&binary, Some(&root), Some(&socket)))
                                .await
                                .map_err(io::Error::other)??;
                            let value = json!({"uri":uri,"host":host,"runId":run});
                            unsafe { uri.as_bytes_mut().fill(0) };
                            value
                        }
                        Control::LiveInfo {} => {
                            let inventory = phone.host().await?;
                            let sessions = native
                                .iter()
                                .map(|proof| {
                                    inventory.sessions.iter().find(|session| target(session) == proof.target
                                        && session.provider == proof.provider && session.native_id == proof.native)
                                        .map(|_| json!({"provider":proof.provider,"target":proof.target,"nativeId":proof.native}))
                                        .ok_or_else(|| failure("development Android original session disappeared"))
                                })
                                .collect::<io::Result<Vec<_>>>()?;
                            json!({"host":host,"runId":run,"sessions":sessions})
                        }
                        Control::Observe { target, marker, receipt } => {
                            if !marker.starts_with("goatr-development-ui-")
                                || receipt.method != ActionMethod::PromptSubmit
                                || receipt.target.as_ref() != Some(&target)
                                || !matches!(receipt.state, ReceiptState::Dispatched | ReceiptState::Succeeded)
                                || receipt.error.is_some()
                            {
                                return Err(failure("development Android prompt receipt/marker mismatch"));
                            }
                            let proof = native.iter().find(|proof| proof.target == target)
                                .ok_or_else(|| failure("development Android prompt changed Target"))?;
                            super::dev_continuity::wait_completed_tool_output(
                                phone,
                                proof,
                                &marker,
                                "development_android_completed_tool_output",
                            )
                            .await?;
                            inputs.current(phone, proof).await?;
                            json!({"verified":true})
                        }
                        Control::Desktop { provider } => {
                            let proof = native.iter().find(|proof| proof.provider == provider)
                                .ok_or_else(|| failure("development Android desktop provider missing"))?;
                            let marker = format!("goatr-development-android-desktop-{}-{:?}", &run.0[..8], provider);
                            inputs.marker(phone, proof, &marker).await?;
                            wait_history(phone, &proof.target, &marker, false).await?;
                            inputs.current(phone, proof).await?;
                            json!({"marker":marker})
                        }
                        Control::DevelopmentReady { device_id, sessions, history } => {
                            if android_only && history.is_none() {
                                return Err(failure("development feedback history observation missing"));
                            }
                            if let Some(rows) = &history
                                && (rows.len() != 2
                                    || [Provider::Omp, Provider::Codex].iter().any(|provider| rows.iter().filter(|row| row.provider == *provider).count() != 1)
                                    || (!android_only && rows.iter().any(|row| row.status != DevelopmentHistoryStatus::Observed)))
                            {
                                return Err(failure("development history observation has invalid providers or status"));
                            }
                            if goatr::admin::call(socket, "devices.list", json!({})).await?["devices"]
                                .as_array()
                                .is_none_or(|devices| !devices.iter().any(|device| device["deviceId"].as_str() == Some(&device_id.0)))
                            {
                                return Err(failure("development Android peer was not actually paired"));
                            }
                            feedback_sessions(native, &sessions)?;
                            for proof in native.iter() {
                                proof.recheck_processes()?;
                            }
                            paired_device = Some(device_id);
                            history_observations = history;
                            complete = true;
                            json!({"retained":true})
                        }
                        _ => return Err(failure("development Android control method is unavailable")),
                    };
                    let bytes = serde_json::to_vec(&response)?;
                    if bytes.len() > 65536 { return Err(failure("development Android control response bound")); }
                    tokio::time::timeout(Duration::from_secs(5), async {
                        stream.write_all(&bytes).await?;
                        stream.write_all(b"\n").await
                    }).await.map_err(|_| failure("development Android control response deadline"))??;
                    Ok::<(), io::Error>(())
                    };
                    match poll_instrumentation_control(child.as_mut(), handler).await {
                        InstrumentationControl::Instrumentation(result) => break result,
                        InstrumentationControl::Handler(result) => result?,
                    }
                }
            }
        }
    }).await;
    drop(child);
    let original_epoch = *cleanup_admitted.borrow();
    let app_cleanup =
        super::camera::stop_app(&l, original_epoch.map(|epoch: AppEpoch| epoch.process))
            .await
            .map(|_| ());
    let reverse_cleanup = if emulator_matches(&l) {
        remove_feedback_reverse(&l, &mapping).await
    } else {
        Ok(())
    };
    let result = preserve_instrumentation_error(
        preserve_instrumentation_error(result, app_cleanup),
        reverse_cleanup,
    )?;
    let development_feedback = class
        == "io.github.trevarj.goatr.GoatrConnectionTest#development_feedback_android_to_desktop";
    let details_display_verified =
        canonical_receipt_display_verified(&result, development_feedback)?;
    if !complete {
        return Err(failure(
            "development Android consumer did not retain the original paired endpoint",
        ));
    }
    let paired_device = paired_device
        .ok_or_else(|| failure("development Android peer identity unavailable after acceptance"))?;
    original_epoch.ok_or_else(|| {
        failure("development Android original app epoch unavailable after acceptance")
    })?;
    if super::camera::app_process(&l).await?.is_some() {
        return Err(failure(
            "development Android process remained after owned instrumentation cleanup",
        ));
    }
    let component = format!("{APP}/io.github.trevarj.goatr.MainActivity");
    let start = adb(&l, &["shell", "am", "start", "-W", "-n", &component]).await?;
    if !start.lines().any(|line| line.trim() == "Status: ok") {
        return Err(failure("development MainActivity launch failed"));
    }
    let app = super::camera::app_process(&l)
        .await?
        .ok_or_else(|| failure("development MainActivity process absent after launch"))?;
    let desktop_attach = development_desktop_attach(
        config,
        root,
        herdr_socket,
        herdr_process,
        native,
        desktop_session,
        desktop_config_home,
    )
    .await?;
    write_json(
        &evidence.join(format!("development-android-{}.json", run.0)),
        &json!({"gate":"development-android-continuity","status":"ready","runId":run,
            "instrumentation":result,"companion":serve.process,"endpointRetained":true,
            "app":{"pid":app.0,"start":app.1},"deviceId":paired_device.clone(),
            "history":history_observations,"rustContinuityPreludeOmitted":android_only,
            "detailsDisplayVerified":details_display_verified,
            "detailsReachability":if details_display_verified {"verified"} else {"unproven"},
            "notClaimed":if details_display_verified {vec!["final Android acceptance"]} else {vec!["canonical receipt details display", "final Android acceptance"]},
            "desktopAttach":desktop_attach.clone(),
            "scope":"actual paired debug APK foreground continuity retained for owner feedback only; final Android acceptance remains required"}),
    )?;
    println!(
        "{}",
        json!({"gate":"development-android-continuity","status":"ready","runId":run,"serial":l.serial,"desktopAttach":desktop_attach})
    );
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let continuation: io::Result<()> = async {
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break Ok(()),
                _ = terminate.recv() => break Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(2)) => {
                    serve.recheck()?;
                    for proof in native.iter() {
                        proof.recheck_processes()?;
                    }
                    let devices = goatr::admin::call(socket, "devices.list", json!({})).await?;
                    if devices["devices"].as_array().is_none_or(|devices| !devices.iter().any(|device| device["deviceId"].as_str() == Some(&paired_device.0))) {
                        break Err(failure("development paired Android peer disappeared"));
                    }
                    if super::camera::app_process(&l).await? != Some(app) {
                        break Err(failure("development MainActivity process exited or changed"));
                    }
                }
            }
        }
    }
    .await;
    let app_cleanup = super::camera::stop_app(&l, Some(app)).await;
    let revoke = goatr::admin::call(socket, "device.revoke", json!({"deviceId":paired_device}))
        .await
        .map(|_| ());
    let serve_cleanup = serve.stop().await;
    write_json(
        &evidence.join(format!("development-android-outcome-{}.json", run.0)),
        &json!({"gate":"development-android-continuity","runId":run,
            "status":if continuation.is_ok() {"stopped"} else {"failed"},
            "appCleanup":app_cleanup.is_ok(),"peerRevoked":revoke.is_ok(),
            "serveStopped":serve_cleanup.is_ok(),"endpointRetainedUntilExit":true}),
    )?;
    continuation?;
    app_cleanup?;
    revoke?;
    serve_cleanup
}

pub(super) struct FeedbackContext<'a> {
    pub binary: &'a Path,
    pub root: &'a Path,
    pub socket: &'a Path,
    pub host: &'a EndpointId,
    pub herdr: &'a ProcessEvidence,
    pub native: &'a [NativeProof],
    pub serve: &'a mut Companion,
    pub root_identity: &'a Value,
    pub package_sha256: &'a str,
}
fn feedback_scope(context: &FeedbackContext<'_>, l: &Lease) -> io::Result<()> {
    if !emulator_matches(l)
        || identity(context.root)? != *context.root_identity
        || herdr::process_evidence(context.herdr.pid)? != *context.herdr
    {
        return Err(failure("original feedback resource identity changed"));
    }
    context.serve.recheck()?;
    for original in context.native {
        original.recheck_processes()?;
    }
    Ok(())
}
fn feedback_sessions(native: &[NativeProof], sessions: &[SessionSummary]) -> io::Result<()> {
    if native.len() != 2
        || sessions.len() != 2
        || !native.iter().any(|p| p.provider == Provider::Omp)
        || !native.iter().any(|p| p.provider == Provider::Codex)
    {
        return Err(failure("feedback requires both original provider sessions"));
    }
    for original in native {
        let matches = sessions
            .iter()
            .filter(|s| {
                target(s) == original.target
                    && s.provider == original.provider
                    && s.native_id == original.native
                    && s.pane_id == original.pane
                    && s.terminal_id == original.terminal
                    && s.workspace_id == original.workspace
                    && s.connectivity == SessionConnectivity::Ready
                    && s.capabilities.history
                    && s.capabilities.prompt
            })
            .count();
        if matches != 1 {
            return Err(failure(
                "feedback changed original native Target or readiness",
            ));
        }
    }
    Ok(())
}
async fn remove_feedback_reverse(l: &Lease, mapping: &str) -> io::Result<()> {
    if !emulator_matches(l) {
        return Err(failure("feedback emulator changed before reverse cleanup"));
    }
    let list = adb(l, &["reverse", "--list"]).await?;
    if list.lines().any(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        fields.len() == 3 && fields[1] == mapping && fields[2] == mapping
    }) {
        adb(l, &["reverse", "--remove", mapping]).await?;
    } else if list
        .lines()
        .any(|line| line.split_whitespace().any(|s| s == mapping))
    {
        return Err(failure("feedback reverse mapping changed; cleanup refused"));
    }
    Ok(())
}
/// A fresh pairing follows the selected checkpoint; any paging deferral remains explicit.
pub(super) async fn feedback(
    args: &clap::ArgMatches,
    evidence: &Path,
    run: &Uuid,
    context: FeedbackContext<'_>,
) -> io::Result<()> {
    let l = super::selected_lease(args, "feedback Android consumer")?;
    let owner_path = evidence.join(format!("ui-owner-{}.json", run.0));
    let owner_source = goatr::setup::ExpectedFile::capture(&owner_path)?;
    let owner: Value = serde_json::from_slice(
        owner_source
            .bytes()
            .ok_or_else(|| failure("passed UI owner absent"))?,
    )
    .map_err(io::Error::other)?;
    if owner["runId"] != json!(run)
        || owner["device"] != check_emulator(&l).await?
        || binary_sha256(context.binary)? != context.package_sha256
    {
        return Err(failure("passed UI emulator/package identity changed"));
    }
    feedback_scope(&context, &l)?;
    let control = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = control.local_addr()?.port();
    let mapping = format!("tcp:{port}");
    if adb(&l, &["reverse", "--list"])
        .await?
        .lines()
        .any(|line| line.split_whitespace().any(|s| s == mapping))
    {
        return Err(failure("feedback reverse mapping already exists"));
    }
    let (admission, mut admitted) = watch::channel(None);
    let mut child = Box::pin(instrument_owned(
        &l,
        "io.github.trevarj.goatr.GoatrConnectionTest#stock_post_pass_fresh_pair_retains_both_original_sessions_for_feedback",
        Some(port),
        false,
        None,
        &admission,
        None,
    ));
    let mut paired = None;
    let mut issued = false;
    let mut verified = false;
    let result = cancel_on_signal(async {
        adb(&l, &["reverse", "--no-rebind", &mapping, &mapping]).await?;
        loop {
            if admitted.borrow().is_none() {
                tokio::select! {
                    result = &mut child => break result,
                    changed = admitted.changed() => { changed.map_err(|_| failure("feedback admission closed"))?; }
                }
                continue;
            }
            tokio::select! {
                result = &mut child => break result,
                incoming = control.accept() => {
                    let (mut stream, peer) = incoming?;
                    if !peer.ip().is_loopback() { continue; }
                    let epoch = (*admitted.borrow()).ok_or_else(|| failure("feedback app epoch absent"))?;
                    let public = epoch.public.ok_or_else(|| failure("feedback signing identity absent"))?;
                    let request = match authenticated_control(&mut stream, &public).await { Ok(request) => request, Err(_) => continue };
                    if super::camera::app_process(&l).await? != Some(epoch.process) {
                        return Err(failure("feedback original instrumentation epoch changed"));
                    }
                    feedback_scope(&context, &l)?;
                    let response = match request {
                        Control::LiveInfo {} => json!({"host":context.host,"runId":run,"sessions":context.native.iter().map(|p|json!({"provider":p.provider,"target":p.target,"nativeId":p.native})).collect::<Vec<_>>()}),
                        Control::LivePair {} if !issued && paired.is_none() => {
                            issued = true;
                            let b = context.binary.to_owned(); let r = context.root.to_owned(); let s = context.socket.to_owned();
                            let mut uri = tokio::task::spawn_blocking(move || terminal_pair(&b, Some(&r), Some(&s))).await.map_err(io::Error::other)??;
                            let response = json!({"uri":uri});
                            unsafe { uri.as_bytes_mut().fill(0); }
                            response
                        }
                        Control::FeedbackPaired {device_id} if issued && paired.is_none() => {
                            let devices = goatr::admin::call(context.socket, "devices.list", json!({})).await?;
                            if !devices["devices"].as_array().is_some_and(|rows| rows.iter().any(|row| row["deviceId"] == json!(device_id))) {
                                return Err(failure("feedback phone is not paired to the original scoped companion"));
                            }
                            paired = Some(device_id);
                            json!({"recorded":true})
                        }
                        Control::FeedbackReady {device_id, sessions, apk_sha256, test_apk_sha256} if issued && paired.as_ref() == Some(&device_id) && !verified => {
                            feedback_sessions(context.native, &sessions)?;
                            if owner["apkSha256"].as_str() != Some(apk_sha256.as_str())
                                || owner["testApkSha256"].as_str() != Some(test_apk_sha256.as_str()) {
                                return Err(failure("installed feedback APK identity differs from passed APKs"));
                            }
                            verified = true;
                            json!({"verified":true})
                        }
                        _ => return Err(failure("control unavailable in post-pass feedback phase")),
                    };
                    let mut bytes = serde_json::to_vec(&response).map_err(io::Error::other)?;
                    bytes.push(b'\n');
                    let sent = tokio::time::timeout(Duration::from_secs(5), stream.write_all(&bytes)).await;
                    bytes.fill(0);
                    sent.map_err(|_| failure("feedback response deadline"))??;
                }
            }
        }
    }).await;
    drop(child);
    let epoch = *admitted.borrow();
    let stopped = super::camera::stop_app(&l, epoch.map(|e| e.process)).await;
    let reverse = remove_feedback_reverse(&l, &mapping).await;
    drop(control);
    drop(admission);
    let mut launched = None;
    let continuation = async {
        stopped?; reverse?;
        let check = result?;
        if !verified { return Err(failure("feedback consumer did not verify both original sessions")); }
        let device_id = paired.as_ref().ok_or_else(|| failure("feedback consumer did not prove fresh pairing"))?;
        owner_source.recheck(&owner_path)?;
        feedback_scope(&context, &l)?;
        if super::camera::app_process(&l).await?.is_some() { return Err(failure("feedback launch refuses a prior APK process")); }
        // This launch owns a new process only after proved absence on the exclusive emulator.
        let component = format!("{APP}/io.github.trevarj.goatr.MainActivity");
        let start = adb(&l, &["shell", "am", "start", "-W", "-n", &component]).await?;
        if !start.lines().any(|line| line.trim() == "Status: ok") {
            return Err(failure("production feedback MainActivity launch failed"));
        }
        launched = super::camera::app_process(&l).await?;
        let app = launched.ok_or_else(|| failure("production feedback process absent after owned launch"))?;
        feedback_scope(&context, &l)?;
        write_json(&evidence.join(format!("feedback-ready-{}.json", run.0)), &json!({
            "gate":feedback_gate(args),"exhaustivePaging":exhaustive_paging(args),"status":"ready","runId":run,"acceptance":format!("stock-{}.json",run.0),
            "scope":"fresh visible pairing and original sessions verified in instrumentation; production MainActivity subsequently launched; foreground only",
            "root":context.root_identity,"packageSha256":context.package_sha256,"companion":context.serve.process,
            "emulator":owner["device"],"app":{"pid":app.0,"start":app.1},"deviceId":device_id,
            "targets":context.native.iter().map(|p| &p.target).collect::<Vec<_>>(),"check":check,
            "supervisor":herdr::process_evidence(std::process::id())?,"pairingSecrets":"memory only","reverseRemoved":true
        }))?;
        println!("{}", json!({"gate":feedback_gate(args),"exhaustivePaging":exhaustive_paging(args),"status":"ready","runId":run,"serial":l.serial}));
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => return Ok(()),
                _ = terminate.recv() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(2)) => {
                    feedback_scope(&context, &l)?;
                    if super::camera::app_process(&l).await? != Some(app) {
                        return Err(failure("feedback production process exited or changed"));
                    }
                }
            }
        }
    }.await;
    // Failure or user interruption ends this scoped feedback lifetime, never desktop work.
    let app_cleanup = super::camera::stop_app(&l, launched).await;
    let revoke = if let Some(device) = &paired {
        goatr::admin::call(context.socket, "device.revoke", json!({"deviceId":device}))
            .await
            .map(|_| ())
    } else {
        Ok(())
    };
    let serve_cleanup = context.serve.stop().await;
    write_json(
        &evidence.join(format!("feedback-outcome-{}.json", run.0)),
        &json!({
            "gate":feedback_gate(args),"exhaustivePaging":exhaustive_paging(args),"runId":run,"status":if continuation.is_ok() {"stopped"} else {"failed"},
            "appCleanup":app_cleanup.is_ok(),"peerRevoked":revoke.is_ok(),"serveStopped":serve_cleanup.is_ok(),
            "desktopRetained":true,"acceptanceUnchanged":true
        }),
    )?;
    continuation?;
    app_cleanup?;
    revoke?;
    serve_cleanup
}

/// Standalone actual installed keyboard/camera consumer: no stock/backend gate is inferred.
pub(super) async fn components(args: &clap::ArgMatches, evidence: &Path) -> io::Result<bool> {
    if args.get_flag("camera") && args.get_flag("ime") {
        return Err(failure("select one standalone component owner"));
    }
    let l = super::selected_lease(args, "standalone UI component")?;
    let device = check_emulator(&l).await?;
    if super::camera::app_process(&l).await?.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "standalone component refuses prior/unowned debug APK process before installation",
        ));
    }
    let apk = fs::canonicalize(
        args.get_one::<PathBuf>("apk")
            .cloned()
            .unwrap_or_else(|| PathBuf::from("app/build/outputs/apk/debug/app-debug.apk")),
    )?;
    let test = fs::canonicalize(args.get_one::<PathBuf>("test-apk").cloned().unwrap_or_else(
        || PathBuf::from("app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"),
    ))?;
    reset_owned_apks(&l).await?;
    install_owned_apk(&l, &apk, false).await?;
    install_owned_apk(&l, &test, true).await?;
    let gate = if args.get_flag("camera") {
        "native-camera"
    } else {
        "real-ime"
    };
    let initial_permission = adb(&l, &["shell", "dumpsys", "package", APP])
        .await?
        .lines()
        .find(|s| s.trim_start().starts_with("android.permission.CAMERA:"))
        .map(|s| s.contains("granted=true"))
        .ok_or_else(|| failure("actual debug-package camera permission state unavailable"))?;
    let result = if args.get_flag("camera") {
        async {
   adb(&l,&["shell","pm","revoke",APP,"android.permission.CAMERA"]).await?;
   adb(&l,&["shell","pm","clear-permission-flags",APP,"android.permission.CAMERA","user-set","user-fixed"]).await?;
   instrument(&l,"io.github.trevarj.goatr.PairingUiTest#deniedCameraLeavesActualPasteAndDisposedQueuedCallbacksCannotAccept",None,false,None).await?;
   adb(&l,&["shell","pm","grant",APP,"android.permission.CAMERA"]).await?;
   super::camera::exercise(&l).await
  }.await
    } else {
        instrument(&l,"io.github.trevarj.goatr.ProviderFormsTest#actualAospImeVisibleKeysEnterBoundedNegativeDecimalAndInteger",None,false,None).await
    };
    if args.get_flag("camera") {
        adb(
            &l,
            &[
                "shell",
                "pm",
                if initial_permission {
                    "grant"
                } else {
                    "revoke"
                },
                APP,
                "android.permission.CAMERA",
            ],
        )
        .await?;
    }
    let result = result?;
    let report = json!({"gate":gate,"status":"pass","device":device,"apkSha256":binary_sha256(&apk)?,"testApkSha256":binary_sha256(&test)?,"result":result,"scope":"actual installed component, not stock providers/live camera authorization/background/real16KiB"});
    let file = evidence.join(format!("{gate}-{}.json", Uuid::random().0));
    write_json(&file, &report)?;
    println!("{}", json!({"gate":gate,"status":"pass","evidence":file}));
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn canonical_receipt_display_observations_are_closed_complete_and_singleton() {
        assert!(
            AutomationSnapshot::parse("receipt_display|omp|canonical_recovery_cleared").is_some()
        );
        for invalid in [
            "receipt_display|none|canonical_recovery_cleared",
            "receipt_display|omp|private",
            "receipt_display|omp|canonical_recovery_cleared|extra",
        ] {
            assert!(AutomationSnapshot::parse(invalid).is_none());
        }
        let mut context = InstrumentationFailureContext::new(
            "io.github.trevarj.goatr.GoatrConnectionTest#development_feedback_android_to_desktop",
        );
        for line in [
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=receipt_display|omp|canonical_recovery_cleared",
            "INSTRUMENTATION_STATUS_CODE: 2",
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=receipt_display|codex|canonical_recovery_cleared",
            "INSTRUMENTATION_STATUS_CODE: 2",
        ] {
            context.observe(line);
        }
        assert!(!context.automation_receipt_display_invalid);
        assert_eq!(context.automation_receipt_display.len(), 2);
        let complete = json!({
            "automation_receipt_display": context.automation_receipt_display.clone(),
        });
        assert!(canonical_receipt_display_verified(&complete, false).unwrap());
        let empty = json!({"automation_receipt_display": {}});
        assert!(canonical_receipt_display_verified(&empty, false).is_err());
        assert!(!canonical_receipt_display_verified(&empty, true).unwrap());
        assert!(
            canonical_receipt_display_verified(
                &json!({"automation_receipt_display": {"omp":"canonical_recovery_cleared"}}),
                true,
            )
            .is_err()
        );
        context.observe("INSTRUMENTATION_STATUS: goatrAutomationSnapshot=receipt_display|omp|canonical_recovery_cleared");
        context.observe("INSTRUMENTATION_STATUS_CODE: 2");
        assert!(context.automation_receipt_display_invalid);
    }
    #[test]
    fn instrumentation_failure_retains_last_closed_automation_phase_before_cleanup() {
        let mut context = InstrumentationFailureContext::new(
            "io.github.trevarj.goatr.GoatrConnectionTest#development_feedback_android_to_desktop",
        );
        for line in [
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=session_open|omp|ready",
            "INSTRUMENTATION_STATUS_CODE: 2",
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=send_wait|omp|selection=1,reservation=0,current=1,send_enabled=0",
            "INSTRUMENTATION_STATUS_CODE: 2",
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=cleanup|none|primary_failed",
            "INSTRUMENTATION_STATUS_CODE: 2",
            "INSTRUMENTATION_STATUS_CODE: -2",
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=cleanup_failure|none|closed",
            "INSTRUMENTATION_STATUS_CODE: 2",
        ] {
            context.observe(line);
        }
        let expected = AutomationSnapshot {
            phase: "send_wait",
            provider: "omp",
            state: "selection=1,reservation=0,current=1,send_enabled=0".to_owned(),
        };
        assert_eq!(context.automation_last, Some(expected.clone()));
        assert_eq!(context.automation_first_failure, Some(expected));
        assert!(!context.automation_truncated);
        assert!(context.automation_cleanup_failed);
        assert!(!context.to_string().contains("goatrAutomationSnapshot"));
    }
    #[test]
    fn stock_await_timeout_snapshot_is_closed_and_precedes_cleanup_failure() {
        let state = "hello=1,inventory_ready=1,source_present=1,exact_reducer=1,mutations_enabled=0,busy=0,refresh_enabled=1,subscription_present=1,exact_selection=1,connectivity=synchronizing,inventory_target=same";
        assert!(AutomationSnapshot::parse(&format!("stock_await_timeout|omp|{state}")).is_some());
        for invalid in [
            format!("stock_await_timeout|none|{state}"),
            format!(
                "stock_await_timeout|omp|{}",
                state.replace("connectivity=synchronizing", "connectivity=private")
            ),
            format!("stock_await_timeout|omp|{state},hello=1"),
            format!(
                "stock_await_timeout|omp|{}",
                state.replacen("hello=1,inventory_ready=1", "inventory_ready=1,hello=1", 1)
            ),
            format!("stock_await_timeout|omp|{state}|extra"),
            format!("stock_await_timeout|omp|{}", "a".repeat(257)),
        ] {
            assert!(AutomationSnapshot::parse(&invalid).is_none(), "{invalid}");
        }
        let mut context = InstrumentationFailureContext::new(
            "io.github.trevarj.goatr.GoatrConnectionTest#development_feedback_android_to_desktop",
        );
        for line in [
            format!(
                "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=stock_await_timeout|omp|{state}"
            ),
            "INSTRUMENTATION_STATUS_CODE: 2".to_owned(),
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=cleanup|none|primary_failed"
                .to_owned(),
            "INSTRUMENTATION_STATUS_CODE: 2".to_owned(),
            "INSTRUMENTATION_STATUS_CODE: -2".to_owned(),
        ] {
            context.observe(&line);
        }
        let expected = AutomationSnapshot {
            phase: "stock_await_timeout",
            provider: "omp",
            state: state.to_owned(),
        };
        assert_eq!(context.automation_last, Some(expected.clone()));
        assert_eq!(context.automation_first_failure, Some(expected));
    }
    #[test]
    fn refresh_timeout_snapshot_is_closed() {
        let state = "selection=1,busy=0,hello=1,inventory_ready=1,mutation_ready=1,source=same,reducer=present,older=true,notice=store_policy:NotReady,failure=compose_timeout";
        assert!(AutomationSnapshot::parse(&format!("refresh_timeout|omp|{state}")).is_some());
        for invalid in [
            format!("refresh_timeout|none|{state}"),
            format!(
                "refresh_timeout|omp|{}",
                state.replace("source=same", "source=private")
            ),
            format!(
                "refresh_timeout|omp|{}",
                state.replace("failure=compose_timeout", "failure=private")
            ),
            format!("refresh_timeout|omp|{state},extra=private"),
            format!("refresh_timeout|omp|{}", "a".repeat(257)),
        ] {
            assert!(AutomationSnapshot::parse(&invalid).is_none(), "{invalid}");
        }
    }
    #[test]
    fn receipt_admission_timeout_snapshot_is_closed_and_precedes_cleanup_failure() {
        let admission_before = "point=before,reservation=same";
        let admission_after = "point=after,reservation=absent";
        let timeout = "foreground=1,busy=0,exact_selection=1,current=0,before_reservation=same,after_reservation=absent,reservation=absent,replacement=absent,focused=same,local_row=same,notice=store_policy:NotReady,admission=post_presend:receipts_unreconciled:NotReady:unknown:unknown";
        assert!(
            AutomationSnapshot::parse(&format!("send_admission|omp|{admission_before}")).is_some()
        );
        assert!(
            AutomationSnapshot::parse(&format!("send_admission|omp|{admission_after}")).is_some()
        );
        assert!(AutomationSnapshot::parse(&format!("receipt_timeout|omp|{timeout}")).is_some());
        assert!(
            AutomationSnapshot::parse(&format!(
                "receipt_timeout|omp|{}",
                timeout.replace(
                    "post_presend:receipts_unreconciled:NotReady:unknown:unknown",
                    "pre_presend:host_not_ready:NotReady:unknown:unknown"
                )
            ))
            .is_some()
        );
        assert!(
            AutomationSnapshot::parse(&format!(
                "receipt_timeout|omp|{}",
                timeout.replace(
                    "post_presend:receipts_unreconciled:NotReady:unknown:unknown",
                    "original_scope:original_absent:NotReady:unknown:unknown"
                )
            ))
            .is_some()
        );
        assert!(
            AutomationSnapshot::parse(&format!(
                "receipt_timeout|omp|{}",
                timeout.replace(
                    "admission=post_presend:receipts_unreconciled:NotReady:unknown:unknown",
                    "admission=none"
                )
            ))
            .is_some()
        );
        assert!(
            AutomationSnapshot::parse(&format!(
                "receipt_timeout|omp|{}",
                timeout.replace(
                    "post_presend:receipts_unreconciled:NotReady:unknown:unknown",
                    "request:inflight_busy:Busy:unknown:unknown"
                )
            ))
            .is_some()
        );
        for invalid in [
            format!("send_admission|none|{admission_before}"),
            format!("send_admission|omp|reservation=same,point=before"),
            format!(
                "receipt_timeout|omp|{}",
                timeout.replace("local_row=same", "local_row=private")
            ),
            format!(
                "receipt_timeout|omp|{}",
                timeout.replace("store_policy:NotReady", "store_policy:private")
            ),
            format!(
                "receipt_timeout|omp|{}",
                timeout.replace("receipts_unreconciled", "private_guard")
            ),
            format!(
                "receipt_timeout|omp|{}",
                timeout.replace(
                    "post_presend:receipts_unreconciled:NotReady:unknown:unknown",
                    "request:private_guard:NotReady:unknown:unknown"
                )
            ),
            format!("receipt_timeout|omp|{timeout},draft=private"),
            format!(
                "receipt_timeout|omp|{}",
                timeout.replacen("foreground=1,busy=0", "busy=0,foreground=1", 1)
            ),
            format!("receipt_timeout|omp|{}", "a".repeat(257)),
        ] {
            assert!(AutomationSnapshot::parse(&invalid).is_none(), "{invalid}");
        }
        let mut context = InstrumentationFailureContext::new(
            "io.github.trevarj.goatr.GoatrConnectionTest#development_feedback_android_to_desktop",
        );
        for line in [
            format!(
                "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=send_admission|omp|{admission_before}"
            ),
            "INSTRUMENTATION_STATUS_CODE: 2".to_owned(),
            format!(
                "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=send_admission|omp|{admission_after}"
            ),
            "INSTRUMENTATION_STATUS_CODE: 2".to_owned(),
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=receipt_polling|omp|state=absent,authoritative=0".to_owned(),
            "INSTRUMENTATION_STATUS_CODE: 2".to_owned(),
            format!(
                "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=receipt_timeout|omp|{timeout}"
            ),
            "INSTRUMENTATION_STATUS_CODE: 2".to_owned(),
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=cleanup|none|primary_failed"
                .to_owned(),
            "INSTRUMENTATION_STATUS_CODE: 2".to_owned(),
            "INSTRUMENTATION_STATUS_CODE: -2".to_owned(),
        ] {
            context.observe(&line);
        }
        let expected = AutomationSnapshot {
            phase: "receipt_timeout",
            provider: "omp",
            state: timeout.to_owned(),
        };
        assert_eq!(context.automation_last, Some(expected.clone()));
        assert_eq!(context.automation_first_failure, Some(expected));
    }
    #[test]
    fn instrumentation_failure_ignores_unknown_malformed_and_oversized_automation_status() {
        let mut context = InstrumentationFailureContext::new(
            "io.github.trevarj.goatr.GoatrConnectionTest#development_feedback_android_to_desktop",
        );
        assert!(
            AutomationSnapshot::parse("receipt_polling|codex|state=dispatching,authoritative=0")
                .is_some()
        );
        assert!(
            AutomationSnapshot::parse(
                "tool_history|omp|rows=many,calls=one,unkeyed=zero,identity=same"
            )
            .is_some()
        );
        assert!(
            AutomationSnapshot::parse(
                "reconnect|codex|rows=one,calls=one,unkeyed=zero,identity=changed"
            )
            .is_some()
        );
        assert!(
            AutomationSnapshot::parse(
                "tool_history|omp|rows=two,calls=one,unkeyed=zero,identity=same"
            )
            .is_none()
        );
        assert!(
            AutomationSnapshot::parse(
                "reconnect|codex|rows=one,calls=one,unkeyed=zero,identity=call-secret"
            )
            .is_none()
        );
        for line in [
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=send_wait|omp|selection=1,reservation=2,current=1,send_enabled=1",
            "INSTRUMENTATION_STATUS_CODE: 2",
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=unknown|omp|ready",
            "INSTRUMENTATION_STATUS_CODE: 2",
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot=send_ready|none|selected_reserved_current_enabled",
            "INSTRUMENTATION_STATUS_CODE: 2",
        ] {
            context.observe(line);
        }
        context.observe(&format!(
            "INSTRUMENTATION_STATUS: goatrAutomationSnapshot={}",
            "a".repeat(257)
        ));
        context.observe("INSTRUMENTATION_STATUS_CODE: 2");
        context.observe("INSTRUMENTATION_STATUS_CODE: -2");
        assert_eq!(context.automation_last, None);
        assert_eq!(context.automation_first_failure, None);
        assert!(!context.automation_truncated);
    }
    #[test]
    fn instrumentation_failure_retains_closed_consumer_location_and_primary_error() {
        let mut context = InstrumentationFailureContext::new(INSTRUMENTATION_SELECTIONS[0].0);
        for line in [
            "INSTRUMENTATION_STATUS: class=io.github.trevarj.goatr.NativeInventoryTest",
            "INSTRUMENTATION_STATUS: test=threeInspectedCreationResetsKeepTheMountedOwnedForm",
            "INSTRUMENTATION_STATUS_CODE: 1",
            "INSTRUMENTATION_STATUS: stack=java.lang.AssertionError: PRIVATE qr/control https://private.invalid/secret",
            "at foreign.NativeInventoryTest.check(NativeInventoryTest.kt:44)",
            "at io.github.trevarj.goatr.NativeInventoryTest.check(PRIVATE.kt:55)",
            "at io.github.trevarj.goatr.NativeInventoryTest.check(NativeInventoryTest.kt:99999999)",
            "at io.github.trevarj.goatr.NativeInventoryTest.check(NativeInventoryTest.kt:84)",
            "at io.github.trevarj.goatr.NativeInventoryTest.check(NativeInventoryTest.kt:91)",
            "INSTRUMENTATION_STATUS_CODE: -2",
            "INSTRUMENTATION_STATUS: class=io.github.trevarj.goatr.NativeLoadTest",
            "INSTRUMENTATION_STATUS: test=PRIVATE-method",
            "INSTRUMENTATION_CODE: -1",
        ] {
            context.observe(line);
        }
        for _ in 0..40 {
            context.observe("INSTRUMENTATION_STATUS_CODE: 0");
        }
        context.observe("INSTRUMENTATION_STATUS_CODE: 1 PRIVATE-payload");
        assert_eq!(context.consumer, "initial_native_components");
        assert_eq!(context.class, Some("NativeInventoryTest"));
        assert_eq!(
            context.method,
            Some("threeInspectedCreationResetsKeepTheMountedOwnedForm")
        );
        assert_eq!(context.location, Some(("NativeInventoryTest.kt", 84)));
        assert_eq!(context.category, "assertion");
        assert_eq!(context.result_code, Some(-1));
        assert_eq!(context.status_codes.len(), 32);
        assert!(context.statuses_truncated);
        let primary = io::Error::other(context);
        let error = preserve_instrumentation_error::<()>(
            Err(primary),
            Err(failure("PRIVATE cleanup failure")),
        )
        .unwrap_err();
        let detail = error
            .get_ref()
            .unwrap()
            .downcast_ref::<InstrumentationFailureContext>()
            .unwrap();
        assert_eq!(detail.location, Some(("NativeInventoryTest.kt", 84)));
        let text = error.to_string();
        assert!(text.len() < 1500);
        assert!(!text.contains("PRIVATE"));
        assert!(!text.contains("https://"));
        let mut unknown = InstrumentationFailureContext::new("PRIVATE arbitrary selected class");
        unknown.observe("INSTRUMENTATION_STATUS: test=PRIVATE");
        unknown.observe("at io.github.trevarj.goatr.NativeLoadTest.check(NativeLoadTest.kt:9)");
        assert_eq!(unknown.selected, "other");
        assert_eq!(unknown.method, None);
        assert_eq!(unknown.location, None);
        assert!(!unknown.to_string().contains("PRIVATE"));
        let stock = InstrumentationFailureContext::new(INSTRUMENTATION_SELECTIONS[5].0);
        assert_eq!(stock.consumer, "stock_foreground");
        assert!(preserve_instrumentation_error(Ok(()), Err(failure("cleanup"))).is_err());
    }

    #[test]
    fn development_instrumentation_cleanup_keeps_the_primary_failure() {
        let error = preserve_instrumentation_error::<()>(
            preserve_instrumentation_error(
                Err(failure("primary instrumentation failure")),
                Err(failure("original app cleanup failure")),
            ),
            Err(failure("reverse cleanup failure")),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "primary instrumentation failure");

        let error = preserve_instrumentation_error(
            preserve_instrumentation_error(Ok(()), Err(failure("original app cleanup failure"))),
            Err(failure("reverse cleanup failure")),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "original app cleanup failure");
    }

    #[test]
    fn owned_apk_reset_rejects_other_packages_and_ambiguous_presence() {
        assert!(owned_package_present("package:io.github.trevarj.goatr.debug\n", APP).unwrap());
        assert!(!owned_package_present("", APP).unwrap());
        assert!(
            owned_package_present("package:io.github.trevarj.goatr.debug.other\n", APP).is_err()
        );
        assert!(owned_package_present("package:io.github.trevarj.goatr.debug\npackage:io.github.trevarj.goatr.debug.test\n", APP).is_err());
        assert!(owned_package_present("", "other.package").is_err());
    }
    use super::*;
    use ring::signature::KeyPair;

    async fn control_attempt(
        key: &ring::signature::EcdsaKeyPair,
        bytes: &[u8],
        fault: u8,
        replay: Option<&[u8]>,
    ) -> (io::Result<Control>, Vec<u8>) {
        let public: [u8; 65] = key.public_key().as_ref().try_into().unwrap();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            authenticated_control(&mut stream, &public).await
        });
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let mut challenge = [0; 32];
        stream.read_exact(&mut challenge).await.unwrap();
        if fault == 1 {
            stream.write_all(bytes).await.unwrap();
            stream.shutdown().await.unwrap();
        } else {
            let mut signing_challenge = challenge;
            if fault == 3 {
                signing_challenge[0] ^= 1;
            }
            let signature = replay.map(Vec::from).unwrap_or_else(|| {
                key.sign(
                    &ring::rand::SystemRandom::new(),
                    &control_message(&signing_challenge, bytes),
                )
                .unwrap()
                .as_ref()
                .to_vec()
            });
            let mut claimed = public;
            if fault == 2 {
                claimed[1] ^= 1;
            }
            let mut sent = bytes.to_vec();
            if fault == 4 {
                sent[0] ^= 1;
            }
            let mut packet = claimed.to_vec();
            packet.push(if fault == 5 {
                73
            } else {
                signature.len() as u8
            });
            packet.extend_from_slice(&signature);
            packet.extend_from_slice(
                &(if fault == 6 {
                    65537u32
                } else {
                    sent.len() as u32
                })
                .to_be_bytes(),
            );
            packet.extend_from_slice(&sent);
            stream.write_all(&packet).await.unwrap();
            stream.shutdown().await.unwrap();
            let mut response = Vec::new();
            // Authentication never emits a pairing URI or application response.
            let _ = stream.read_to_end(&mut response).await;
            assert!(response.is_empty());
            return (server.await.unwrap(), signature);
        }
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response).await;
        assert!(response.is_empty());
        (server.await.unwrap(), Vec::new())
    }

    #[tokio::test]
    async fn private_control_rejects_foreign_unsigned_unknown_key_tamper_replay_and_unclosed_bytes()
    {
        let rng = ring::rand::SystemRandom::new();
        let material = ring::signature::EcdsaKeyPair::generate_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            &rng,
        )
        .unwrap();
        let key = ring::signature::EcdsaKeyPair::from_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            material.as_ref(),
            &rng,
        )
        .unwrap();
        let bytes = br#"{"method":"live_pair"}"#;
        let (result, signature) = control_attempt(&key, bytes, 0, None).await;
        assert!(matches!(result, Ok(Control::LivePair {})));
        assert!(matches!(
            control_attempt(&key, br#"{"method":"live_info"}"#, 0, None)
                .await
                .0,
            Ok(Control::LiveInfo {})
        ));
        assert!(matches!(
            control_attempt(&key, br#"{"method":"restart"}"#, 0, None)
                .await
                .0,
            Ok(Control::Restart {})
        ));
        assert!(matches!(
            control_attempt(&key, br#"{"method":"recovery_info"}"#, 0, None)
                .await
                .0,
            Ok(Control::RecoveryInfo {})
        ));
        assert!(matches!(
            control_attempt(&key, br#"{"method":"complete"}"#, 0, None)
                .await
                .0,
            Ok(Control::Complete {})
        ));
        assert!(matches!(
            control_attempt(&key, br#"{"method":"desktop","provider":"omp"}"#, 0, None)
                .await
                .0,
            Ok(Control::Desktop {
                provider: Provider::Omp
            })
        ));
        // Feedback ownership registration still uses the same signed, closed control envelope.
        let feedback =
            serde_json::to_vec(&json!({"method":"feedback_paired","device_id":Uuid::random()}))
                .unwrap();
        assert!(matches!(
            control_attempt(&key, &feedback, 0, None).await.0,
            Ok(Control::FeedbackPaired { .. })
        ));
        assert!(control_attempt(&key, &feedback, 1, None).await.0.is_err());
        for fault in 1..=6 {
            assert!(control_attempt(&key, bytes, fault, None).await.0.is_err());
        }
        assert!(
            control_attempt(&key, bytes, 0, Some(&signature))
                .await
                .0
                .is_err()
        );
        for (label, bytes) in [
            (
                "duplicate method",
                br#"{"method":"live_pair","method":"live_pair"}"#.as_slice(),
            ),
            (
                "escaped duplicate method",
                br#"{"method":"live_pair","m\u0065thod":"live_pair"}"#.as_slice(),
            ),
            (
                "live_pair extra member",
                br#"{"method":"live_pair","unowned":true}"#.as_slice(),
            ),
            (
                "live_info extra member",
                br#"{"method":"live_info","unowned":true}"#.as_slice(),
            ),
            (
                "restart extra member",
                br#"{"method":"restart","unowned":true}"#.as_slice(),
            ),
            (
                "recovery_info extra member",
                br#"{"method":"recovery_info","unowned":true}"#.as_slice(),
            ),
            (
                "complete extra member",
                br#"{"method":"complete","unowned":true}"#.as_slice(),
            ),
            (
                "duplicate payload member",
                br#"{"method":"desktop","provider":"omp","provider":"omp"}"#.as_slice(),
            ),
            (
                "payload extra member",
                br#"{"method":"desktop","provider":"omp","unowned":true}"#.as_slice(),
            ),
            (
                "unknown method",
                br#"{"method":"not_a_control"}"#.as_slice(),
            ),
            (
                "feedback registration extra member",
                br#"{"method":"feedback_paired","device_id":"11111111-1111-4111-8111-111111111111","unowned":true}"#.as_slice(),
            ),
            (
                "feedback readiness missing proof",
                br#"{"method":"feedback_ready","device_id":"11111111-1111-4111-8111-111111111111"}"#.as_slice(),
            ),
        ] {
            assert!(
                control_attempt(&key, bytes, 0, None).await.0.is_err(),
                "{label}"
            );
        }
    }

    #[tokio::test]
    async fn development_control_cancels_pending_handler_when_instrumentation_fails() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        struct PendingHandler(Arc<AtomicBool>);
        impl Future for PendingHandler {
            type Output = io::Result<()>;

            fn poll(
                self: Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Self::Output> {
                std::task::Poll::Pending
            }
        }
        impl Drop for PendingHandler {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let mut child = Box::pin(async { Err::<Value, _>(failure("instrumentation failed")) });
        let cancelled = Arc::new(AtomicBool::new(false));
        match poll_instrumentation_control(child.as_mut(), PendingHandler(Arc::clone(&cancelled)))
            .await
        {
            InstrumentationControl::Instrumentation(result) => assert!(result.is_err()),
            InstrumentationControl::Handler(_) => {
                panic!("pending handler won instrumentation failure")
            }
        }
        assert!(cancelled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn owned_instrumentation_completion_reaps_owned_child_after_deadline() {
        const CHILD: &str = "GOATR_OWNED_INSTRUMENTATION_COMPLETION_CHILD";
        if std::env::var_os(CHILD).is_some() {
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "live::ui::tests::owned_instrumentation_completion_reaps_owned_child_after_deadline",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let status =
            finish_owned_instrumentation_with_deadline(&mut child, Duration::from_millis(50))
                .await
                .unwrap();
        assert!(!status.success());
        assert!(child.try_wait().unwrap().is_some());
    }

    #[tokio::test]
    async fn signal_cancels_actual_terminal_input_barrier() {
        const CHILD: &str = "GOATR_CANCEL_BARRIER_CHILD";
        if std::env::var_os(CHILD).is_some() {
            cancel_on_signal(human("actual private PTY cancellation regression"))
                .await
                .unwrap_err();
            return;
        }
        use std::os::fd::FromRawFd;
        let fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
        assert!(fd >= 0);
        let _master = unsafe { File::from_raw_fd(fd) };
        assert_eq!(unsafe { libc::grantpt(fd) }, 0);
        assert_eq!(unsafe { libc::unlockpt(fd) }, 0);
        let mut name = [0i8; 256];
        assert_eq!(
            unsafe { libc::ptsname_r(fd, name.as_mut_ptr(), name.len()) },
            0
        );
        let path = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }
            .to_str()
            .unwrap();
        let slave = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "live::ui::tests::signal_cancels_actual_terminal_input_barrier",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdin(Stdio::from(slave))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let original = herdr::process_evidence(child.id().unwrap()).unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut line = String::new();
                assert!(output.read_line(&mut line).await.unwrap() > 0);
                assert!(line.len() <= 4096);
                if line.contains("human-native-barrier") {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(herdr::process_evidence(original.pid).unwrap(), original);
        assert_eq!(unsafe { libc::kill(original.pid as i32, libc::SIGTERM) }, 0);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
}
