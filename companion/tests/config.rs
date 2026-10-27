// SPDX-License-Identifier: GPL-3.0-or-later
use goatr::{config::*, protocol::*};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(format!(".goatr-config-{}", Uuid::random().0));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn text(&self) -> String {
        let wrapper = self.0.join("herdr");
        fs::write(&wrapper, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        format!(
            "version=1\nruntime_dir={:?}\nruntime_dir_agent={:?}\npush_allowed_origins=[\"https://ntfy.sh\"]\n[executables]\nherdr={:?}\n[[projects]]\nid={:?}\nlabel=\"project\"\npath={:?}\n",
            self.0,
            self.0,
            wrapper,
            Uuid::random().0,
            self.0
        )
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn exact_version_types_unknown_keys_duplicate_ids_and_local_only_admin() {
    let root = Root::new();
    let text = root.text();
    let config = Config::parse(&text).unwrap();
    config.preflight().unwrap();
    assert!(config.projects()[0].available);
    for wrong in [
        text.replacen("version=1", "version=2", 1),
        text.replacen("version=1", "version=\"1\"", 1),
        format!("admin_socket=\"/tmp/admin.sock\"\n{text}"),
        text.replace("label=\"project\"", "label=\"project\"\nunknown=true"),
        text.replace("version=1", "version=1\nversion=1"),
        text.replace("https://ntfy.sh", "https://ntfy.sh/path"),
        text.replace("https://ntfy.sh", "https://user@ntfy.sh"),
        text.replace("https://ntfy.sh", "http://ntfy.sh"),
        text.replace("https://ntfy.sh", "https://ntfy.sh?token=x"),
        text.replace("https://ntfy.sh", "https://ntfy.sh#fragment"),
    ] {
        assert!(Config::parse(&wrong).is_err(), "{wrong}");
    }
    let repeated = format!(
        "{text}\n[[projects]]\nid={:?}\nlabel=\"duplicate\"\npath={:?}\n",
        config.projects[0].id.0, root.0
    );
    assert!(Config::parse(&repeated).is_err());
    assert!(
        Config::parse(&text.replace(
            &format!("runtime_dir_agent={:?}", root.0),
            "runtime_dir_agent=\"~/Workspace/.goatr\""
        ))
        .is_err()
    );
}
#[test]
fn pairing_name_is_optional_bounded_and_validated_during_parse_and_serialization() {
    let root = Root::new();
    let text = root.text();
    let default = Config::parse(&text).unwrap();
    assert!(default.pairing_name.is_none());
    let omitted = default.serialized().unwrap();
    assert!(!omitted.contains("pairing_name"));
    assert!(Config::parse(&omitted).unwrap().pairing_name.is_none());
    let label = "Goatr MVP 123e4567-e89b-42d3-a456-426614174000";
    let named = Config::parse(&format!("pairing_name={label:?}\n{text}")).unwrap();
    let roundtrip = Config::parse(&named.serialized().unwrap()).unwrap();
    assert_eq!(roundtrip.pairing_name, Some(Label::new(label).unwrap()));
    assert_eq!(roundtrip.version, default.version);
    assert_eq!(roundtrip.executables.herdr, default.executables.herdr);
    for invalid in ["".to_owned(), "a".repeat(129), "bad\nname".to_owned(), "bad\u{7f}name".to_owned()] {
        let quoted = serde_json::to_string(&invalid).unwrap();
        assert!(Config::parse(&format!("pairing_name={quoted}\n{text}")).is_err());
        let mut forged = Config::parse(&text).unwrap();
        forged.pairing_name = Some(Label(invalid));
        assert!(forged.serialized().is_err());
    }
}

#[test]
fn private_config_symlink_mode_socket_both_views_and_missing_project() {
    let root = Root::new();
    let text = root.text();
    let path = root.0.join("config.toml");
    fs::write(&path, &text).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    Config::load(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Config::load(&path).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = root.0.join("alias");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(Config::load(&alias).is_err());
    let suffix = "x".repeat(108);
    assert!(socket_path(&root.0.join(&suffix)).is_err());
    let owner = format!(
        "{text}\n[[codex_owners]]\nid={:?}\nsocket_host={:?}\nsocket_agent={:?}\nbindings_dir={:?}\nunit=\"goatr-codex.service\"\n",
        Uuid::random().0,
        root.0.join("c.sock"),
        PathBuf::from(format!("/{suffix}")),
        root.0
    );
    assert!(Config::parse(&owner).is_err());
    let mut config = Config::parse(&text).unwrap();
    config.projects[0].path = root.0.join("gone");
    config.preflight().unwrap();
    assert!(!config.projects()[0].available);
    assert_eq!(
        config.project(&config.projects[0].id).unwrap_err(),
        ErrorCode::NotReady
    );
}
#[test]
fn installed_outer_wrapper_is_retained_and_never_invoked_by_preflight() {
    let root = Root::new();
    let text = root.text();
    let mut config = Config::parse(&text).unwrap();
    let outer = root.0.join("omp");
    let inner = root.0.join("raw");
    fs::write(&inner, "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(&inner, fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&inner, &outer).unwrap();
    config.executables.omp = Some(outer.clone());
    config.preflight().unwrap();
    assert_eq!(config.wrapper(Provider::Omp), Some(outer.as_path()));
    assert_eq!(setup_project(root.0.to_str().unwrap()).unwrap(), root.0);
    fs::set_permissions(&inner, fs::Permissions::from_mode(0o722)).unwrap();
    assert!(config.preflight().is_err());
}

#[test]
fn configured_existing_socket_entries_and_cross_kind_owner_ids_fail_closed() {
    let root = Root::new();
    let text = root.text();
    let mut config = Config::parse(&text).unwrap();
    let path = root.0.join("c.sock");
    config.herdr_sockets = vec![path.clone()];
    config.preflight().unwrap();
    fs::write(&path, "not a Unix socket").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(config.preflight().is_err());
    fs::remove_file(&path).unwrap();
    let socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    config.preflight().unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(config.preflight().is_err());
    drop(socket);
    let duplicate = format!(
        "{text}\n[[codex_owners]]\nid={:?}\nsocket_host={:?}\nsocket_agent={:?}\nbindings_dir={:?}\nunit=\"goatr-codex.service\"\n",
        config.projects[0].id.0, path, path, root.0
    );
    assert!(Config::parse(&duplicate).is_err());
}
#[test]
fn native_codex_rendezvous_shape_is_config_trust_only_and_never_a_herdr_socket() {
    let root = Root::new();
    let mut config = Config::parse(&root.text()).unwrap();
    config.executables.codex = Some(root.0.join("herdr"));
    let host = root.0.join("c.sock");
    let agent = PathBuf::from("/r/c.sock");
    let target = codex_socket_target(&agent).unwrap();
    config.codex_owners = vec![CodexOwner {
        id: Uuid::random(),
        socket_host: host.clone(),
        socket_agent: agent.clone(),
        bindings_dir: root.0.clone(),
        unit: "goatr-rendezvous.service".into(),
    }];
    std::os::unix::fs::symlink(&target, &host).unwrap();
    // A publisher/process/root/native handshake is still required by the runtime.
    config.preflight().unwrap();
    assert_eq!(config.backend_reason(Provider::Codex).unwrap(), None);
    config.herdr_sockets = vec![host.clone()];
    assert!(
        config.preflight().is_err(),
        "Herdr direct-socket guards are unchanged"
    );
    config.herdr_sockets.clear();
    fs::remove_file(&host).unwrap();
    std::os::unix::fs::symlink(target.with_file_name("foreign"), &host).unwrap();
    assert!(config.preflight().is_err());
    fs::remove_file(&host).unwrap();
    std::os::unix::fs::symlink(&target, &host).unwrap();
    let hard_link = root.0.join("second-alias");
    fs::hard_link(&host, &hard_link).unwrap();
    assert!(
        config.preflight().is_err(),
        "aliased rendezvous entries are not stock publication"
    );
    fs::remove_file(&hard_link).unwrap();
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(config.preflight().is_err());
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn disappeared_optional_omp_root_preserves_shared_and_codex_configuration() {
    let root = Root::new();
    let registry = root.0.join("registry");
    fs::create_dir(&registry).unwrap();
    fs::set_permissions(&registry, fs::Permissions::from_mode(0o700)).unwrap();
    let text = root.text();
    let wrapper = root.0.join("herdr");
    let text = format!(
        "omp_registry_dir={registry:?}\n{text}\n[[codex_owners]]\nid={:?}\nsocket_host={:?}\nsocket_agent={:?}\nbindings_dir={:?}\nunit=\"goatr-codex.service\"\n",
        Uuid::random().0,
        root.0.join("c.sock"),
        root.0.join("c.sock"),
        root.0,
    )
    .replace(
        &format!("herdr={wrapper:?}"),
        &format!("herdr={wrapper:?}\nomp={wrapper:?}\ncodex={wrapper:?}"),
    );
    let path = root.0.join("config.toml");
    fs::write(&path, &text).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    Config::load(&path).unwrap();
    fs::remove_dir(&registry).unwrap();
    let loaded =
        Config::load(&path).expect("absent OMP root must not block the shared/Codex owner");
    assert_eq!(loaded.runtime_dir, root.0);
    assert_eq!(loaded.wrapper(Provider::Codex), Some(wrapper.as_path()));
    assert_eq!(loaded.codex_owners.len(), 1);
    assert!(loaded.projects()[0].available);
    assert_eq!(
        loaded.backend_reason(Provider::Omp).unwrap(),
        Some("Configured OMP registry directory is unavailable"),
    );
    assert_eq!(loaded.backend_reason(Provider::Codex).unwrap(), None);
    fs::create_dir(&registry).unwrap();
    fs::set_permissions(&registry, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        Config::load(&path).is_err(),
        "unsafe existing OMP root must remain fatal"
    );
}

#[test]
fn selected_toolchain_store_executable_keeps_outer_wrapper_and_rejects_mutable_parent() {
    let root = Root::new();
    let mut config = Config::parse(&root.text()).unwrap();
    let selected = PathBuf::from(env!("CARGO"));
    executable(&selected).expect("selected installed toolchain must have executable trust proof");
    let outer = root.0.join("selected-wrapper");
    std::os::unix::fs::symlink(&selected, &outer).unwrap();
    config.executables.herdr = outer.clone();
    config.preflight().unwrap();
    assert_eq!(config.executables.herdr, outer);
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o777)).unwrap();
    let unsafe_parent = executable(&config.executables.herdr);
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        unsafe_parent.is_err(),
        "readonly target must not excuse a mutable outer parent"
    );
}

#[test]
fn scoped_shell_probe_requires_explicit_local_boolean_consent() {
    let root = Root::new();
    let text = root.text();
    assert!(!Config::parse(&text).unwrap().allow_scoped_shell_probe);
    assert!(
        Config::parse(&format!("allow_scoped_shell_probe=true\n{text}"))
            .unwrap()
            .allow_scoped_shell_probe
    );
    for value in ["1", "\"true\"", "[]", "{}"] {
        assert!(Config::parse(&format!("allow_scoped_shell_probe={value}\n{text}")).is_err());
    }
    let config = Config::parse(&format!("allow_scoped_shell_probe=false\n{text}")).unwrap();
    assert!(config.projects()[0].available);
}

#[test]
fn codex_creation_selector_is_explicit_and_never_list_order_or_inventory_filter() {
    let root = Root::new();
    let base = root.text();
    let first = Uuid::random();
    let second = Uuid::random();
    let owner = |id: &Uuid, suffix: &str| {
        format!(
            "\n[[codex_owners]]\nid={:?}\nsocket_host={:?}\nsocket_agent={:?}\nbindings_dir={:?}\nunit={:?}\n",
            id.0,
            root.0.join(format!("{suffix}.sock")),
            root.0.join(format!("{suffix}.sock")),
            root.0,
            format!("goatr-{suffix}.service")
        )
    };
    let zero = Config::parse(&base).unwrap();
    assert!(zero.creation_codex_owner().is_err());
    let sole = Config::parse(&format!("{base}{}", owner(&first, "first"))).unwrap();
    assert_eq!(sole.creation_codex_owner().unwrap().id, first);
    let both = format!(
        "{base}{}{}",
        owner(&first, "first"),
        owner(&second, "second")
    );
    let ambiguous = Config::parse(&both).unwrap();
    ambiguous.preflight().unwrap();
    assert!(ambiguous.creation_codex_owner().is_err());
    let selected =
        Config::parse(&format!("default_codex_owner_id={:?}\n{both}", second.0)).unwrap();
    selected.preflight().unwrap();
    assert_eq!(selected.creation_codex_owner().unwrap().id, second);
    assert_eq!(selected.codex_owners.len(), 2);
    assert_eq!(selected.codex_owners[0].id, first);
    let reversed = format!(
        "default_codex_owner_id={:?}\n{base}{}{}",
        second.0,
        owner(&second, "second"),
        owner(&first, "first")
    );
    assert_eq!(
        Config::parse(&reversed)
            .unwrap()
            .creation_codex_owner()
            .unwrap()
            .id,
        second
    );
    for wrong in [
        format!("default_codex_owner_id={:?}\n{both}", Uuid::random().0),
        format!("default_codex_owner_id=\"not-a-uuid\"\n{both}"),
        format!(
            "default_codex_owner_id={:?}\ndefault_codex_owner_id={:?}\n{both}",
            first.0, second.0
        ),
        format!(
            "{base}{}{}",
            owner(&first, "first"),
            owner(&first, "duplicate")
        ),
    ] {
        assert!(Config::parse(&wrong).is_err());
    }
    let mut changed = selected;
    changed.default_codex_owner_id = Some(Uuid::random());
    assert!(changed.preflight().is_err());
}

#[test]
fn unavailable_unselected_owner_is_not_selected_creation_or_an_inventory_allowlist() {
    let root = Root::new();
    let mut config = Config::parse(&root.text()).unwrap();
    let wrapper = root.0.join("herdr");
    config.executables.codex = Some(wrapper);
    let selected = Uuid::random();
    config.default_codex_owner_id = Some(selected.clone());
    config.codex_owners = vec![
        CodexOwner {
            id: Uuid::random(),
            socket_host: root.0.join("missing").join("c.sock"),
            socket_agent: root.0.join("missing").join("c.sock"),
            bindings_dir: root.0.join("missing"),
            unit: "goatr-unavailable.service".into(),
        },
        CodexOwner {
            id: selected.clone(),
            socket_host: root.0.join("c.sock"),
            socket_agent: root.0.join("c.sock"),
            bindings_dir: root.0.clone(),
            unit: "goatr-selected.service".into(),
        },
    ];
    config.preflight().unwrap();
    assert_eq!(config.backend_reason(Provider::Codex).unwrap(), None);
    assert_eq!(config.creation_codex_owner().unwrap().id, selected);
    assert_eq!(config.codex_owners.len(), 2);
    fs::create_dir(root.0.join("missing")).unwrap();
    fs::set_permissions(root.0.join("missing"), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        config.preflight().is_err(),
        "unselected does not excuse an unsafe existing configured entry"
    );
}

#[test]
fn read_review_grants_are_explicit_canonical_and_independent_of_creation_or_terminal_control() {
    let root=Root::new();let text=root.text();let mut config=Config::parse(&text).unwrap();
    let id=config.projects[0].id.clone();let label=config.projects[0].label.clone();
    assert!(!config.projects[0].allow_read_tools);assert!(!config.allow_terminal_control);
    assert!(config.projects()[0].available);
    assert!(config.set_project_read_tools(&root.0,true).unwrap());assert!(!config.set_project_read_tools(&root.0,true).unwrap());
    assert!(config.projects[0].allow_read_tools);assert!(!config.allow_terminal_control);
    assert_eq!(config.projects[0].id,id);assert_eq!(config.projects[0].label,label);assert_eq!(config.projects[0].path,root.0);
    assert!(!config.approve_project(root.0.to_str().unwrap()).unwrap());assert!(config.projects[0].allow_read_tools);
    let unconfigured=root.0.join("unconfigured");fs::create_dir(&unconfigured).unwrap();
    assert!(config.set_project_read_tools(&unconfigured,true).is_err());assert_eq!(config.projects.len(),1);
    assert!(config.set_project_read_tools(&root.0,false).unwrap());assert!(config.projects()[0].available);
    let explicit=Config::parse(&text.replace("label=\"project\"","label=\"project\"\nallow_read_tools=true")).unwrap();assert!(explicit.projects[0].allow_read_tools);
    assert!(Config::parse(&text.replace("label=\"project\"","label=\"project\"\nallow_read_tools=\"true\"")).is_err());
    let alias=root.0.join("root-alias");std::os::unix::fs::symlink(&root.0,&alias).unwrap();
    let duplicate=format!("{text}\n[[projects]]\nid={:?}\nlabel=\"same canonical object\"\npath={:?}\n",Uuid::random().0,alias);
    assert!(Config::parse(&duplicate).and_then(|config|config.preflight()).is_err());
}

