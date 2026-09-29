//! Profile compatibility, application locking, and private persistent settings.
#![allow(clippy::unwrap_used)]

use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use yettel_cwmp::{
    domain::{
        export::Export,
        profile::{CredentialsSource, Profile},
        secret::Secret,
        serial::Serial,
    },
    error::Error,
    store::Store,
};

fn profile() -> Profile {
    Profile::new(
        "SYN123456".parse().unwrap(),
        "02:11:22:33:44:55".parse().unwrap(),
        "synthetic-wlan".into(),
    )
    .unwrap()
}

fn directory(root: &Path, serial: &Serial) -> std::path::PathBuf {
    root.join("devices").join(serial.as_ref())
}

fn saved_settings() -> Value {
    json!({
        "internet": {
            "protocol": "PPPoE",
            "vlan_id": 710,
            "mtu": 1492,
            "username": "synthetic-user",
            "password": "synthetic-password"
        }
    })
}

#[test]
fn creation_reuses_identical_profile_and_rejects_key_or_identity_conflicts() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let mut profile = profile();
    assert!(store.create(&profile).unwrap() == profile);
    let path = directory(root.path(), &profile.serial).join("profile.json");
    let original = fs::read(&path).unwrap();
    assert!(store.create(&profile).unwrap() == profile);
    profile.password = Secret::new("synthetic-different-key");
    assert!(matches!(store.create(&profile), Err(Error::ProfileExists)));
    assert_eq!(fs::read(&path).unwrap(), original);
    profile.router_mac = "02:11:22:33:44:66".parse().unwrap();
    assert!(matches!(
        store.create(&profile),
        Err(Error::ProfileConflict)
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn listing_sorts_saved_routers_and_keeps_damaged_profiles_selectable() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    assert!(store.list().unwrap().is_empty());
    let first = profile();
    let mut second = profile();
    second.serial = "SYN654321".parse().unwrap();
    store.create(&second).unwrap();
    store.create(&first).unwrap();
    fs::create_dir(root.path().join("devices/SYN999999")).unwrap();
    fs::write(
        directory(root.path(), &first.serial).join("profile.json"),
        b"{broken synthetic profile",
    )
    .unwrap();
    assert_eq!(store.list().unwrap(), vec![first.serial, second.serial]);
}

#[test]
fn deleting_a_damaged_profile_removes_its_settings_and_keeps_other_routers() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let first = profile();
    let mut second = profile();
    second.serial = "SYN654321".parse().unwrap();
    store.create(&first).unwrap();
    store.create(&second).unwrap();
    let export: Export = serde_json::from_value(saved_settings()).unwrap();
    let export_path = store.save_export(&first.serial, &export).unwrap();
    let profile_path = directory(root.path(), &first.serial).join("profile.json");
    fs::write(&profile_path, b"{broken synthetic profile").unwrap();
    store.delete(&first.serial).unwrap();
    assert!(!profile_path.exists());
    assert!(!export_path.exists());
    assert!(matches!(
        store.load(&first.serial),
        Err(Error::ProfileMissing)
    ));
    assert!(store.load(&second.serial).unwrap() == second);
    assert_eq!(store.list().unwrap(), vec![second.serial]);
}

#[test]
fn global_lock_blocks_second_process_until_store_is_dropped() {
    const PROBE: &str = "YETTEL_SYNTHETIC_STORE_LOCK_PROBE";
    if let Some(root) = std::env::var_os(PROBE) {
        assert!(matches!(
            Store::open(Path::new(&root)),
            Err(Error::AlreadyRunning)
        ));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    assert!(matches!(
        Store::open(root.path()),
        Err(Error::AlreadyRunning)
    ));
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "global_lock_blocks_second_process_until_store_is_dropped",
        ])
        .env(PROBE, root.path())
        .output()
        .unwrap();
    assert!(result.status.success(), "synthetic lock probe failed");
    drop(store);
    assert!(Store::open(root.path()).is_ok());
}

#[test]
fn version_two_ignores_legacy_fields_and_reads_unknown_origin_as_server() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let profile = profile();
    store.create(&profile).unwrap();
    let path = directory(root.path(), &profile.serial).join("profile.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["provider"] = json!("synthetic-legacy-provider");
    value["device"] = json!("synthetic-legacy-device");
    value["acs_url"] = json!("https://old.example.invalid/synthetic");
    value["connect_notice_accepted"] = json!(true);
    value["credentials_source"] = json!("unknown");
    let original = serde_json::to_vec(&value).unwrap();
    fs::write(&path, &original).unwrap();
    let loaded = store.load(&profile.serial).unwrap();
    assert_eq!(loaded.credentials_source, CredentialsSource::Server);
    assert_eq!(loaded.password, profile.password);
    assert_eq!(fs::read(&path).unwrap(), original);
    store.save(&loaded).unwrap();
    let saved: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(saved["credentials_source"], "server");
    for key in ["provider", "device", "acs_url", "connect_notice_accepted"] {
        assert!(saved.get(key).is_none(), "obsolete field {key}");
    }
}

#[test]
fn invalid_profile_versions_contents_and_identity_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let profile = profile();
    store.create(&profile).unwrap();
    let path = directory(root.path(), &profile.serial).join("profile.json");
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut cases = vec![json!({"version": 3, "future": true})];
    for version in [json!(0), json!(true), json!(1.0), Value::Null] {
        let mut invalid = original.clone();
        invalid["version"] = version;
        cases.push(invalid);
    }
    let mut missing_version = original.clone();
    missing_version.as_object_mut().unwrap().remove("version");
    cases.push(missing_version);
    let mut wrong_identity = original;
    wrong_identity["serial"] = json!("SYN654321");
    cases.push(wrong_identity);
    for invalid in cases {
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(matches!(
            store.load(&profile.serial),
            Err(Error::ProfileInvalid)
        ));
    }
    for invalid in [b"{broken".as_slice(), b"[]", b"\xff"] {
        fs::write(&path, invalid).unwrap();
        assert!(matches!(
            store.load(&profile.serial),
            Err(Error::ProfileInvalid)
        ));
    }
}

#[test]
fn missing_profile_does_not_create_a_device() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let serial = profile().serial;
    assert!(matches!(store.load(&serial), Err(Error::ProfileMissing)));
    assert!(!directory(root.path(), &serial).exists());
}

#[test]
fn saved_export_round_trips_offline_without_rewriting_the_profile() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let profile = profile();
    store.create(&profile).unwrap();
    assert!(matches!(
        store.load_export(&profile.serial),
        Err(Error::NoSavedSettings)
    ));
    assert_eq!(store.export_modified(&profile.serial).unwrap(), None);
    let path = directory(root.path(), &profile.serial).join("profile.json");
    let before = fs::read(&path).unwrap();
    let export: Export = serde_json::from_value(saved_settings()).unwrap();
    let saved_path = store.save_export(&profile.serial, &export).unwrap();
    let (read_path, loaded) = store.load_export(&profile.serial).unwrap();
    assert!(read_path.is_absolute());
    assert_eq!(read_path, saved_path);
    assert_eq!(serde_json::to_value(loaded).unwrap(), saved_settings());
    assert!(store.export_modified(&profile.serial).unwrap().is_some());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn relative_root_returns_the_same_absolute_export_path() {
    let cwd = std::env::current_dir().unwrap();
    let root = tempfile::tempdir_in(&cwd).unwrap();
    let relative_root = root.path().strip_prefix(&cwd).unwrap();
    assert!(!relative_root.is_absolute());

    let store = Store::open(relative_root).unwrap();
    let profile = profile();
    store.create(&profile).unwrap();
    let export: Export = serde_json::from_value(saved_settings()).unwrap();
    let saved_path = store.save_export(&profile.serial, &export).unwrap();
    let (read_path, loaded) = store.load_export(&profile.serial).unwrap();

    assert!(saved_path.is_absolute());
    assert_eq!(read_path, saved_path);
    assert!(read_path.exists());
    assert_eq!(serde_json::to_value(loaded).unwrap(), saved_settings());
}

#[test]
fn saved_export_requires_complete_typed_internet_settings() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let profile = profile();
    store.create(&profile).unwrap();
    let path = directory(root.path(), &profile.serial).join("extracted-credentials.json");
    let mut cases = vec![json!({"internet": {}})];
    for field in ["protocol", "vlan_id", "mtu", "username", "password"] {
        let mut missing = saved_settings();
        missing["internet"].as_object_mut().unwrap().remove(field);
        cases.push(missing);
        let mut wrong_type = saved_settings();
        wrong_type["internet"][field] = json!(false);
        cases.push(wrong_type);
    }
    for (field, value) in [
        ("protocol", json!("SyntheticWAN")),
        ("username", json!("")),
        ("password", json!("")),
        ("vlan_id", json!(-1)),
        ("mtu", json!(65536)),
    ] {
        let mut invalid = saved_settings();
        invalid["internet"][field] = value;
        cases.push(invalid);
    }
    // Damaged settings are received again; the profile and its credentials stay usable.
    for invalid in cases {
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(matches!(
            store.load_export(&profile.serial),
            Err(Error::NoSavedSettings)
        ));
    }
    fs::write(&path, b"{broken synthetic export").unwrap();
    assert!(matches!(
        store.load_export(&profile.serial),
        Err(Error::NoSavedSettings)
    ));
    assert!(store.load(&profile.serial).is_ok());
}

#[cfg(unix)]
#[test]
fn created_paths_are_private_and_existing_directory_modes_are_unchanged() {
    use std::os::unix::fs::PermissionsExt;

    let parent = tempfile::tempdir().unwrap();
    fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let root = parent.path().join("new-parent/state");
    let store = Store::open(&root).unwrap();
    let profile = profile();
    store.create(&profile).unwrap();
    let export: Export = serde_json::from_value(saved_settings()).unwrap();
    let export_path = store.save_export(&profile.serial, &export).unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(parent.path()), 0o755);
    for path in [
        parent.path().join("new-parent"),
        root.clone(),
        root.join("devices"),
        directory(&root, &profile.serial),
    ] {
        assert_eq!(mode(&path), 0o700);
    }
    for path in [
        root.join(".lock"),
        directory(&root, &profile.serial).join("profile.json"),
        export_path,
    ] {
        assert_eq!(mode(&path), 0o600);
    }
    drop(store);
    for path in [
        &root,
        &root.join("devices"),
        &directory(&root, &profile.serial),
    ] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let store = Store::open(&root).unwrap();
    store.save(&profile).unwrap();
    for path in [
        root.clone(),
        root.join("devices"),
        directory(&root, &profile.serial),
    ] {
        assert_eq!(mode(&path), 0o755);
    }
}
