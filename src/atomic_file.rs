use anyhow::{Context, Result};
use std::{
    fs::{self, File, OpenOptions, Permissions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempGuard(Option<PathBuf>);

impl TempGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

/// Atomically replace one private file in an owner-only directory.
pub(crate) fn replace(path: &Path, bytes: &[u8], directory_mode: u32) -> Result<()> {
    replace_with_directory_policy(path, bytes, directory_mode, true)
}

/// Atomically replace one private file, making missing directories private
/// while preserving permissions on an existing parent directory.
pub(crate) fn replace_preserving_directories(
    path: &Path,
    bytes: &[u8],
    directory_mode: u32,
) -> Result<()> {
    replace_with_directory_policy(path, bytes, directory_mode, false)
}

fn replace_with_directory_policy(
    path: &Path,
    bytes: &[u8],
    directory_mode: u32,
    rewrite_existing_parent: bool,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .context("atomic file path has no parent directory")?;
    let parent_existed = parent.exists();
    fs::create_dir_all(parent)
        .with_context(|| format!("creating directory {}", parent.display()))?;
    if rewrite_existing_parent || !parent_existed {
        fs::set_permissions(parent, Permissions::from_mode(directory_mode))
            .with_context(|| format!("setting directory permissions on {}", parent.display()))?;
    }

    let file_name = path
        .file_name()
        .context("atomic file path has no file name")?
        .to_string_lossy();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        sequence
    ));
    let mut guard = TempGuard(Some(temporary.clone()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .with_context(|| format!("creating temporary file in {}", parent.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("writing temporary file in {}", parent.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing temporary file in {}", parent.display()))?;
    drop(file);
    fs::set_permissions(&temporary, Permissions::from_mode(0o600))
        .with_context(|| format!("setting file permissions in {}", parent.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))?;
    guard.disarm();
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("syncing directory {}", parent.display()))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct TestDirectory(PathBuf);

    impl TestDirectory {
        pub(crate) fn new(label: &str) -> Self {
            let sequence = DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ancs-bridge-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self(path)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
