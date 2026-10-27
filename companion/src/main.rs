// SPDX-License-Identifier: GPL-3.0-or-later
use clap::{Arg, Command, value_parser};
use goatr::{
    admin,
    pairing::*,
    protocol::*,
    state::State,
    storage::{Storage, StorageOwner},
};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal, Write},
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};
fn cli() -> Command {
    let socket = Arg::new("admin-socket")
        .long("admin-socket")
        .value_name("PATH")
        .value_parser(value_parser!(PathBuf))
        .global(true)
        .help("Exact absolute private local admin socket; no fallback");
    let config = Arg::new("config")
        .long("config")
        .value_name("PATH")
        .value_parser(value_parser!(PathBuf))
        .help("Exact private local version1 TOML; defaults to XDG configuration");
    Command::new("goatr")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Foreground native companion; paired devices have host-wide authority")
        .arg(socket)
        .subcommand_required(true)
        .subcommand(
            Command::new("codex-relay")
                .hide(true)
                .about("Independent internal Codex Unix relay; never starts a provider")
                .arg(
                    Arg::new("binding")
                        .long("binding")
                        .required(true)
                        .value_name("PATH")
                        .value_parser(value_parser!(PathBuf)),
                ),
        )
        .subcommand(
            Command::new("codex-owner")
                .hide(true)
                .about("Internal capability-dropped Codex owner launcher")
                .arg(
                    Arg::new("record")
                        .long("record")
                        .required(true)
                        .value_name("PATH")
                        .value_parser(value_parser!(PathBuf)),
                ),
        )
        .subcommand(
            Command::new("serve")
                .about("Own retained identity and SQLite; run in the foreground")
                .arg(config.clone().conflicts_with("transport-smoke"))
                .arg(
                    Arg::new("transport-smoke")
                        .long("transport-smoke")
                        .value_parser(["direct", "relay"])
                        .hide(true)
                        .requires_all(["local-hints-file", "admin-socket"]),
                )
                .arg(
                    Arg::new("local-hints-file")
                        .long("local-hints-file")
                        .value_parser(value_parser!(PathBuf))
                        .hide(true)
                )
                .arg(Arg::new("owned-omp-relay-fd").long("owned-omp-relay-fd").hide(true).value_parser(value_parser!(i32)).requires_all(["admin-socket", "config"]).conflicts_with("transport-smoke")),
        )
        .subcommand(
            Command::new("setup")
                .about("Explicit private configuration/project approval; separately consented owned serve unit")
                .arg(config)
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PATH")
                        .action(clap::ArgAction::Append),
                )
                .args(["runtime-dir", "runtime-dir-agent", "herdr", "omp", "codex", "omp-registry-dir", "omp-blob-root", "codex-owner-socket", "codex-owner-socket-agent", "codex-bindings-dir"].map(|name| {let arg=Arg::new(name).long(name).value_name("PATH").value_parser(value_parser!(PathBuf));if matches!(name,"codex-owner-socket-agent"|"codex-bindings-dir"){arg.requires("codex-owner-socket")}else{arg}}))
                .arg(Arg::new("omp-session-root").long("omp-session-root").value_name("PATH").value_parser(value_parser!(PathBuf)).action(clap::ArgAction::Append))
                .arg(Arg::new("omp-skills-package-root").long("omp-skills-package-root").value_name("PATH").value_parser(value_parser!(PathBuf)).help("Select exact qualified public OMP extension source for future owned launches"))
                .arg(Arg::new("skill-document-root").long("skill-document-root").value_name("PATH").value_parser(value_parser!(PathBuf)).action(clap::ArgAction::Append).conflicts_with("clear-skill-document-roots").help("Offer separate bounded enabled-Skill document-read consent for these roots"))
                .arg(Arg::new("clear-skill-document-roots").long("clear-skill-document-roots").action(clap::ArgAction::SetTrue))
                .arg(Arg::new("allow-omp-controls").long("allow-omp-controls").action(clap::ArgAction::SetTrue).conflicts_with("disable-omp-controls").help("Offer separate public original-session model/thinking/delivery control consent"))
                .arg(Arg::new("disable-omp-controls").long("disable-omp-controls").action(clap::ArgAction::SetTrue))
                .arg(Arg::new("allow-project-read").long("allow-project-read").value_name("PATH").value_parser(value_parser!(PathBuf)).action(clap::ArgAction::Append).conflicts_with("disable-project-read").help("Offer separate subtree read/export and confined Git-review consent for an already configured project"))
                .arg(Arg::new("disable-project-read").long("disable-project-read").value_name("PATH").value_parser(value_parser!(PathBuf)).action(clap::ArgAction::Append))
                .arg(Arg::new("codex-unit").long("codex-unit").value_name("UNIT").requires("codex-owner-socket"))
                .arg(Arg::new("codex-owner").long("codex-owner").value_name("UUID").help("Select an existing configured owner for creation only"))
                .arg(Arg::new("allow-scoped-shell-probe").long("allow-scoped-shell-probe").action(clap::ArgAction::SetTrue).conflicts_with("disable-scoped-shell-probe").help("Offer separate exact fresh-root-only read-only probe consent"))
                .arg(Arg::new("disable-scoped-shell-probe").long("disable-scoped-shell-probe").action(clap::ArgAction::SetTrue))
                .arg(Arg::new("allow-terminal-control").long("allow-terminal-control").action(clap::ArgAction::SetTrue).conflicts_with("disable-terminal-control").help("Offer separate consent for explicit stock non-agent terminal control"))
                .arg(Arg::new("disable-terminal-control").long("disable-terminal-control").action(clap::ArgAction::SetTrue))
                .arg(Arg::new("serve-unit").long("serve-unit").action(clap::ArgAction::SetTrue).help("Offer independent owned user-unit installation, then separate start consent"))
        )
        .subcommand(
            Command::new("pair")
                .about("Explicitly open a 300-second pairing window in this terminal"),
        )
        .subcommand(
            Command::new("devices").about("List paired devices through the running companion"),
        )
        .subcommand(
            Command::new("revoke")
                .about("Revoke a device and wait for entered dispatch boundaries")
                .arg(Arg::new("device-id").required(true).value_name("DEVICE_ID")),
        )
}
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
fn publish_hints(path: &Path, public: &goatr::transport::LocalHints) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("invalid hints parent"))?;
    private_dir(parent, false)?;
    let temporary = parent.join(format!(".goatr-hints-{}", Uuid::random().0));
    let mut hints = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)?;
    let mut published = false;
    let result = (|| {
        hints.write_all(&serde_json::to_vec(public)?)?;
        hints.sync_all()?;
        let directory = File::open(parent)?;
        let source = std::ffi::CString::new(
            temporary
                .file_name()
                .expect("temporary filename")
                .as_bytes(),
        )?;
        let destination = std::ffi::CString::new(
            path.file_name()
                .ok_or_else(|| io::Error::other("invalid hints filename"))?
                .as_bytes(),
        )?;
        // SAFETY: live directory FD and NUL-terminated relative names. No-replace also rejects symlinks.
        if unsafe {
            libc::renameat2(
                directory.as_raw_fd(),
                source.as_ptr(),
                directory.as_raw_fd(),
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        published = true;
        directory.sync_all()
    })();
    if !published {
        fs::remove_file(&temporary)?;
    }
    result
}
async fn run(matches: clap::ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
    let (name, sub) = matches.subcommand().expect("required command");
    if name == "codex-relay" {
        goatr::codex_relay::run(sub.get_one::<PathBuf>("binding").expect("required binding"))
            .await?;
        return Ok(());
    }
    if name == "setup" {
        if matches.get_one::<PathBuf>("admin-socket").is_some() {
            return Err("setup uses the production admin path; --admin-socket is only for serve/pair/devices/revoke".into());
        }
        let path = sub
            .get_one::<PathBuf>("config")
            .cloned()
            .map(Ok)
            .unwrap_or_else(goatr::config::default_path)?;
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = goatr::setup::run(sub, &path) => result?,
            _ = tokio::signal::ctrl_c() => return Err("setup cancelled; committed stages remain durable, no pairing/provider effects".into()),
            _ = terminate.recv() => return Err("setup cancelled; committed stages remain durable, no pairing/provider effects".into()),
        }
        return Ok(());
    }
    let explicit = matches.get_one::<PathBuf>("admin-socket").cloned();
    let socket = explicit
        .clone()
        .map(Ok)
        .unwrap_or_else(admin::default_socket)?;
    if name == "serve" {
        // umask also protects SQLite's new WAL/SHM and the initial bind before chmod.
        // SAFETY: process-local file creation mask, set before any identity/provider operation.
        unsafe {
            libc::umask(0o077);
        }
        let config = if sub.get_one::<String>("transport-smoke").is_some() {
            None
        } else {
            let path = sub
                .get_one::<PathBuf>("config")
                .cloned()
                .map(Ok)
                .unwrap_or_else(goatr::config::default_path)?;
            Some(goatr::config::Config::load(&path)?)
        };
        let owned_omp_relay = match sub.get_one::<i32>("owned-omp-relay-fd") {
            Some(fd) => {
                let config = config
                    .as_ref()
                    .ok_or("owned relay requires actual configuration")?;
                let relay = goatr::backends::omp::Relay::inherited(
                    *fd,
                    &config.runtime_dir.join("omp-relay.owner"),
                    &config.runtime_dir,
                )?;
                // Close only the exact successfully validated inherited descriptor; the
                // retained duplicate belongs to Relay, never to provider/desktop lifetime.
                if unsafe { libc::close(*fd) } != 0 {
                    return Err(io::Error::last_os_error().into());
                }
                Some(relay)
            }
            None => None,
        };
        let admin = admin::AdminSocket::bind(socket)?;
        let identity = HostIdentity::acquire(state_root()?)?;
        let storage = Storage::open(
            &identity.root.join("state.sqlite3"),
            identity.receipt_key,
            now_ms(),
            uid(),
        )?;
        let boot = storage.boot_id.clone();
        let store = StorageOwner::new(storage)?;
        let mut builder = goatr::endpoint_builder(identity.secret.clone());
        match sub.get_one::<String>("transport-smoke").map(String::as_str) {
            Some("direct") => {
                builder = builder
                    .relay_mode(iroh::RelayMode::Disabled)
                    .clear_address_lookup();
            }
            Some("relay") => {
                builder = builder.clear_ip_transports();
            }
            _ => {}
        }
        let endpoint = builder.bind().await?;
        if sub
            .get_one::<String>("transport-smoke")
            .is_some_and(|mode| mode == "relay")
        {
            tokio::time::timeout(std::time::Duration::from_secs(45), endpoint.online()).await?;
        }
        if let Some(path) = sub.get_one::<PathBuf>("local-hints-file") {
            let public = goatr::transport::LocalHints {
                endpoint_addr: endpoint.addr(),
                package_version: env!("CARGO_PKG_VERSION").into(),
                companion_lock_sha256: goatr::transport::lock_sha256().into(),
                core_iroh: "1.3.0".into(),
            };
            publish_hints(path, &public)?;
        }
        // Backend availability is honest until the topology/provider owners supply real bindings.
        let backends = [Provider::Omp, Provider::Codex]
            .into_iter()
            .map(|provider| Backend {
                provider,
                version: "not bound".into(),
                availability: Availability::Unavailable,
                reason: Some("No proven native provider binding is installed".into()),
            })
            .collect();
        let security = Security::new(
            store,
            Arc::new(State::default()),
            EndpointId(endpoint.id().to_string()),
            boot,
            backends,
            config.as_ref().and_then(|config| config.pairing_name.clone()),
        );
        let topology = match config {
            Some(config) => Some(
                goatr::herdr::Topology::new(
                    config,
                    security.storage.clone(),
                    security.state.clone(),
                    security.host.clone(),
                )
                .await
                .map_err(|code| format!("Topology preflight rejected: {code:?}"))?,
            ),
            None => None,
        };
        let (stop, rx) = tokio::sync::watch::channel(false);
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let result = {
            let admin_job = admin.serve(security.clone(), rx.clone());
            let codex_invalidation_reporter =
                goatr::transport::owned_codex_transport_capture(owned_omp_relay.as_ref());
            let transport = goatr::transport::serve(
                &endpoint,
                security.clone(),
                topology.clone(),
                owned_omp_relay,
                codex_invalidation_reporter.clone(),
                rx.clone(),
            );
            let topology_job = async {
                if let Some(owner) = &topology {
                    owner.clone().run(rx.clone()).await
                } else {
                    // The isolated transport-only smoke has explicitly disabled topology.
                    let mut stop = rx.clone();
                    loop {
                        if *stop.borrow() {
                            break;
                        }
                        if stop.changed().await.is_err() {
                            break;
                        }
                    }
                    Ok(())
                }
            };
            tokio::pin!(admin_job, transport, topology_job);
            let (selected_result, finished, winner) = tokio::select! {
                result = &mut admin_job => (result, 1, goatr::transport::CodexTransportWinner::Admin),
                result = &mut transport => (result, 2, goatr::transport::CodexTransportWinner::Transport),
                result = &mut topology_job => (result, 3, goatr::transport::CodexTransportWinner::Topology),
                _ = tokio::signal::ctrl_c() => (Ok(()), 0, goatr::transport::CodexTransportWinner::CtrlC),
                _ = terminate.recv() => (Ok(()), 0, goatr::transport::CodexTransportWinner::Sigterm),
            };
            if let Some(reporter) = &codex_invalidation_reporter {
                reporter.selected(winner, (finished != 0).then_some(&selected_result));
            }
            let _ = stop.send(true);
            // Drain every sibling before cleanup; a selected error must not drop topology
            // while it settles the current creation receipt.
            let admin_result = if finished == 1 { Ok(()) } else { admin_job.await };
            let transport_result = if finished == 2 { Ok(()) } else { transport.await };
            let topology_result = if finished == 3 { Ok(()) } else { topology_job.await };
            if let Some(reporter) = &codex_invalidation_reporter {
                if finished != 1 && admin_result.is_err() {
                    reporter.sibling_error(goatr::transport::CodexTransportWinner::Admin, &admin_result);
                } else if finished != 2 && transport_result.is_err() {
                    reporter.sibling_error(goatr::transport::CodexTransportWinner::Transport, &transport_result);
                } else if finished != 3 && topology_result.is_err() {
                    reporter.sibling_error(goatr::transport::CodexTransportWinner::Topology, &topology_result);
                }
            }
            admin_result
                .and(transport_result)
                .and(topology_result)
                .and(selected_result)
        };
        security
            .shutdown()
            .await
            .map_err(|_| "entered provider boundary could not settle")?;
        goatr::close_endpoint(&endpoint).await?;
        drop(endpoint);
        drop(admin);
        drop(topology);
        let security = Arc::into_inner(security).ok_or("runtime security owner still retained")?;
        let store = Arc::into_inner(security.storage).ok_or("SQLite owner still retained")?;
        store.close().await?;
        drop(identity);
        result?;
        return Ok(());
    }
    if name == "pair" && !io::stdout().is_terminal() {
        return Err("pair requires an explicitly requested local terminal; QR/token are never emitted into captured logs".into());
    }
    let (method, params) = match name {
        "pair" => ("pair.open", json!({})),
        "devices" => ("devices.list", json!({})),
        "revoke" => {
            let id = Uuid::new(
                sub.get_one::<String>("device-id")
                    .expect("required ID")
                    .clone(),
            )?;
            ("device.revoke", json!({"deviceId":id}))
        }
        _ => unreachable!(),
    };
    let value = match admin::call(&socket, method, params).await {
        Ok(v) => v,
        Err(e) => {
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) {
                let instruction = explicit.as_ref().map_or(String::new(), |p| {
                    format!(" --admin-socket {}", quote(&p.to_string_lossy()))
                });
                let binary = std::env::current_exe()?;
                eprintln!(
                    "goatr is not running. In a local terminal run: {} serve{instruction}",
                    quote(&binary.to_string_lossy())
                );
            }
            return Err(e.into());
        }
    };
    match name {
        "pair" => {
            let pair: PairOpen = serde_json::from_value(value)?;
            PairInvite::parse(&pair.uri).map_err(|_| "invalid pairing reply")?;
            println!(
                "Pairing grants inventory, history, control and notifications for ALL supported same-user sessions on this host. Public relays/discovery observe connection metadata, not plaintext. Expires in 300 seconds."
            );
            if let Ok(qr) = qrcode::QrCode::new(pair.uri.as_bytes()) {
                println!(
                    "{}",
                    qr.render::<qrcode::render::unicode::Dense1x2>()
                        .dark_color(qrcode::render::unicode::Dense1x2::Light)
                        .light_color(qrcode::render::unicode::Dense1x2::Dark)
                        .build()
                );
            } else {
                println!("QR exceeds encoder capacity; paste the complete URI below.");
            }
            println!("{}", pair.uri);
        }
        "devices" => {
            let devices: admin::Devices = serde_json::from_value(value)?;
            for d in devices.devices {
                println!(
                    "{}\t{}\t{}\t{}",
                    d.device_id.0, d.device_name.0, d.paired_at.0, d.endpoint_id.0
                );
            }
        }
        "revoke" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Revoked {
                revoked: bool,
            }
            let result: Revoked = serde_json::from_value(value)?;
            println!(
                "{}",
                if result.revoked {
                    "revoked"
                } else {
                    "already absent"
                }
            );
        }
        _ => unreachable!(),
    }
    Ok(())
}
fn codex_owner_boundary<T>(operation: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    // This is deliberately before record parsing, subprocesses, or Tokio worker creation.
    #[cfg(target_os = "linux")]
    goatr::codex_relay::drop_inherited_capabilities()?;
    #[cfg(not(target_os = "linux"))]
    return Err(io::Error::other(
        "Codex owner launcher requires Linux capability controls",
    ));
    operation()
}
fn codex_owner_main(matches: &clap::ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
    codex_owner_boundary(|| {
        if matches.get_one::<PathBuf>("admin-socket").is_some() {
            return Err(io::Error::other(
                "codex-owner has no companion/admin lifetime",
            ));
        }
        let (_, sub) = matches.subcommand().expect("required command");
        goatr::backends::codex::exec_owner(
            sub.get_one::<PathBuf>("record").expect("required record"),
        )
    })?;
    Ok(())
}
fn main() -> ExitCode {
    let matches = cli().get_matches();
    let is_codex_relay = matches.subcommand_name() == Some("codex-relay");
    let result = if matches.subcommand_name() == Some("codex-owner") {
        codex_owner_main(&matches)
    } else {
        (|| -> Result<(), Box<dyn std::error::Error>> {
            if is_codex_relay {
                if matches.get_one::<PathBuf>("admin-socket").is_some() {
                    return Err("codex-relay has no companion/admin lifetime".into());
                }
                // Linux capabilities are per-thread; future runtime workers must inherit zero sets.
                #[cfg(target_os = "linux")]
                goatr::codex_relay::drop_inherited_capabilities()?;
            }
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed building the Runtime")
                .block_on(run(matches))
        })()
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if !is_codex_relay || !goatr::codex_relay::take_startup_diagnostic_reported()
            {
                eprintln!("goatr: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::{
            fs::{DirBuilderExt, MetadataExt},
            process::CommandExt,
        },
        process::{Command, Stdio},
    };

    #[test]
    fn owner_launcher_capability_drop_precedes_fixture_exec() {
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "grep '^Cap\\(Inh\\|Prm\\|Eff\\|Amb\\):' /proc/self/status",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped());
        // SAFETY: this isolated test child runs the same drop before its harmless exec.
        unsafe {
            command.pre_exec(|| codex_owner_boundary(|| Ok(())));
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        let lines = std::str::from_utf8(&output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 4);
        for line in lines {
            assert!(line.ends_with("0000000000000000"), "{line}");
        }
    }

    #[test]
    fn hints_publication_is_complete_private_and_never_replaces_entries() {
        let root = std::env::temp_dir().join(format!("goatr-hints-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let path = root.join("hints.json");
        let public = goatr::transport::LocalHints {
            endpoint_addr: iroh::EndpointAddr::new(iroh::SecretKey::from_bytes(&[7; 32]).public()),
            package_version: env!("CARGO_PKG_VERSION").into(),
            companion_lock_sha256: goatr::transport::lock_sha256().into(),
            core_iroh: "1.3.0".into(),
        };
        publish_hints(&path, &public).unwrap();
        private_file(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let decoded: goatr::transport::LocalHints = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.endpoint_addr, public.endpoint_addr);
        let inode = fs::symlink_metadata(&path).unwrap().ino();
        assert_eq!(
            publish_hints(&path, &public).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), inode);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let link = root.join("alias");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(
            publish_hints(&link, &public).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}
