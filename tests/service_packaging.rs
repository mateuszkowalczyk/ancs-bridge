use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::symlink,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

const BINARY: &str = env!("CARGO_BIN_EXE_ancs-bridge");
const UNIT: &str = include_str!("../packaging/ancs-bridge.service");
const STAGE_INSTALL: &str = include_str!("../packaging/stage-install.sh");
static DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temporary_directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ancs-bridge-service-{label}-{}-{}",
        std::process::id(),
        DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn relative_files(root: &Path, directory: &Path, files: &mut BTreeSet<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            relative_files(root, &path, files);
        } else {
            files.insert(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
}

#[test]
fn unit_contract_is_exact_private_and_payload_free() {
    for required in [
        "[Unit]",
        "[Service]",
        "Type=simple",
        "ExecStart=/usr/bin/ancs-bridge daemon",
        "Restart=on-failure",
        "RestartSec=3s",
        "RuntimeDirectory=ancs-bridge",
        "RuntimeDirectoryMode=0700",
        "UMask=0077",
        "NoNewPrivileges=true",
        "PrivateTmp=true",
        "[Install]",
        "WantedBy=default.target",
    ] {
        assert!(
            UNIT.lines().any(|line| line == required),
            "missing {required}"
        );
    }

    let normalized = UNIT.to_ascii_lowercase();
    for forbidden in [
        "user=root",
        "environment=",
        "environmentfile=",
        "execstartpre=",
        "execstartpost=",
        "/bin/sh",
        "sudo",
        "notification title",
        "notification body",
    ] {
        assert!(
            !normalized.contains(forbidden),
            "forbidden unit content: {forbidden}"
        );
    }
}

#[test]
fn staged_install_has_only_final_artifacts_and_no_state_mutation() {
    let root = temporary_directory("staging");
    let status = Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/stage-install.sh"))
        .arg(&root)
        .env("ANCS_BRIDGE_BINARY", BINARY)
        .status()
        .unwrap();
    assert!(status.success());

    let mut files = BTreeSet::new();
    relative_files(&root, &root, &mut files);
    assert_eq!(
        files,
        BTreeSet::from([
            PathBuf::from("usr/bin/ancs-bridge"),
            PathBuf::from("usr/lib/systemd/user/ancs-bridge.service"),
            PathBuf::from("usr/share/licenses/ancs-bridge/LICENSE"),
        ])
    );
    assert_eq!(
        fs::metadata(root.join("usr/bin/ancs-bridge"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    for relative in [
        "usr/lib/systemd/user/ancs-bridge.service",
        "usr/share/licenses/ancs-bridge/LICENSE",
    ] {
        assert_eq!(
            fs::metadata(root.join(relative))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
    }
    assert_eq!(
        fs::read_to_string(root.join("usr/lib/systemd/user/ancs-bridge.service")).unwrap(),
        UNIT
    );

    let normalized = STAGE_INSTALL.to_ascii_lowercase();
    for forbidden in [
        "systemctl",
        "bluetoothctl",
        "wireplumber",
        "config.toml",
        "setup --jsonl",
    ] {
        assert!(
            !normalized.contains(forbidden),
            "staging helper mutates user state: {forbidden}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn staging_helper_rejects_root_path_aliases() {
    let root = temporary_directory("root-alias");
    let root_link = root.join("root-link");
    symlink("/", &root_link).unwrap();
    for destination in [
        PathBuf::from("/./"),
        PathBuf::from("//"),
        PathBuf::from("/tmp/.."),
        root_link,
    ] {
        let output = Command::new("bash")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/stage-install.sh"))
            .arg(destination)
            .env("ANCS_BRIDGE_BINARY", "/definitely/missing/ancs-bridge")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("DESTDIR must be a non-root"));
    }
    fs::remove_dir_all(root).unwrap();
}
