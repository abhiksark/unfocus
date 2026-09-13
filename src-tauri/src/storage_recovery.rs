use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static QUARANTINE_FILE_ID: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
pub(crate) static TEST_QUARANTINE_FAILURES: std::sync::Mutex<Vec<PathBuf>> =
    std::sync::Mutex::new(Vec::new());
#[cfg(test)]
static TEST_REPLACEMENT_FAILURES: std::sync::Mutex<Vec<PathBuf>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
struct TestReplacementBarrier {
    path: PathBuf,
    started: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
static TEST_REPLACEMENT_BARRIERS: std::sync::Mutex<Vec<TestReplacementBarrier>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
struct TestQuarantineValidationBarrier {
    path: PathBuf,
    started: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
static TEST_QUARANTINE_VALIDATION_BARRIER: std::sync::Mutex<
    Option<TestQuarantineValidationBarrier>,
> = std::sync::Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StorageLoadStatus {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StorageRecovery {
    None,
    Retry,
    RetryOrStartNew,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StorageFailureCategory {
    Read,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageLoadHealth {
    pub(crate) status: StorageLoadStatus,
    pub(crate) recovery: StorageRecovery,
}

impl StorageLoadHealth {
    pub(crate) const fn available() -> Self {
        Self {
            status: StorageLoadStatus::Available,
            recovery: StorageRecovery::None,
        }
    }

    pub(crate) const fn unavailable(category: StorageFailureCategory) -> Self {
        Self {
            status: StorageLoadStatus::Unavailable,
            recovery: match category {
                StorageFailureCategory::Read => StorageRecovery::Retry,
                StorageFailureCategory::Invalid => StorageRecovery::RetryOrStartNew,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageDiagnostic {
    pub(crate) status: StorageLoadStatus,
    pub(crate) recovery: StorageRecovery,
    pub(crate) category: Option<StorageFailureCategory>,
    pub(crate) error: Option<String>,
}

impl StorageDiagnostic {
    pub(crate) fn available() -> Self {
        Self {
            status: StorageLoadStatus::Available,
            recovery: StorageRecovery::None,
            category: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalSnapshot<T> {
    pub(crate) load_health: StorageLoadHealth,
    pub(crate) data: Option<T>,
}

impl<T> LocalSnapshot<T> {
    pub(crate) fn available(data: T) -> Self {
        Self {
            load_health: StorageLoadHealth::available(),
            data: Some(data),
        }
    }

    pub(crate) fn unavailable(category: StorageFailureCategory) -> Self {
        Self {
            load_health: StorageLoadHealth::unavailable(category),
            data: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LoadFailure {
    pub(crate) category: StorageFailureCategory,
    pub(crate) technical_error: String,
}

impl LoadFailure {
    pub(crate) fn read(error: impl Into<String>) -> Self {
        Self {
            category: StorageFailureCategory::Read,
            technical_error: error.into(),
        }
    }

    pub(crate) fn invalid(error: impl Into<String>) -> Self {
        Self {
            category: StorageFailureCategory::Invalid,
            technical_error: error.into(),
        }
    }

    pub(crate) fn health(&self) -> StorageLoadHealth {
        StorageLoadHealth::unavailable(self.category)
    }

    pub(crate) fn diagnostic(&self) -> StorageDiagnostic {
        StorageDiagnostic {
            status: StorageLoadStatus::Unavailable,
            recovery: self.health().recovery,
            category: Some(self.category),
            error: Some(self.technical_error.clone()),
        }
    }
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "storage path has no parent"))?;
    fs::File::open(parent)?.sync_all()
}

// Windows does not expose a portable directory-sync operation through std.
// The file and replacement temporary file are still synced; directory sync is
// deliberately best-effort unsupported rather than making recovery fail.
#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn sync_parent_after_replacement(path: &Path) {
    // Once rename/ReplaceFile succeeds, rolling back is neither atomic nor
    // safe. Attempt the durability barrier, but do not turn a completed
    // replacement into a reported recovery failure if directory sync itself
    // is unsupported or fails.
    if let Err(error) = sync_parent_directory(path) {
        eprintln!("could not sync the local-storage directory after replacement: {error}");
    }
}

fn fail_replacement_if_injected(_path: &Path) -> io::Result<()> {
    #[cfg(test)]
    if TEST_REPLACEMENT_FAILURES.lock().is_ok_and(|mut targets| {
        targets
            .iter()
            .position(|target| target == _path)
            .map(|index| targets.remove(index))
            .is_some()
    }) {
        return Err(io::Error::other("injected canonical replacement failure"));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn inject_replacement_failure(path: PathBuf) {
    TEST_REPLACEMENT_FAILURES
        .lock()
        .expect("replacement failure hook lock")
        .push(path);
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn replace_file_atomically(temp_path: &Path, path: &Path) -> io::Result<()> {
    fail_replacement_if_injected(path)?;
    fs::rename(temp_path, path)?;
    sync_parent_after_replacement(path);
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn replace_file_atomically(temp_path: &Path, path: &Path) -> io::Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};

    fail_replacement_if_injected(path)?;
    match fs::rename(temp_path, path) {
        Ok(()) => {
            sync_parent_after_replacement(path);
            return Ok(());
        }
        Err(error) if !path.exists() => return Err(error),
        Err(_) => {}
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }

    let replaced: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let replacement: Vec<u16> = temp_path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both paths are owned, NUL-terminated UTF-16 buffers that remain
    // alive for the call; optional pointers are null. ReplaceFileW either
    // atomically replaces the destination or reports failure.
    let replaced_ok = unsafe {
        ReplaceFileW(
            replaced.as_ptr(),
            replacement.as_ptr(),
            ptr::null(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if replaced_ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        sync_parent_after_replacement(path);
        Ok(())
    }
}

/// Re-read the canonical pathname at the last possible point before an
/// atomic replacement and require it to still contain the bytes that were
/// quarantined. This narrows concurrent external-repair races without a file
/// locking dependency. The app serializes its own recovery, but arbitrary
/// external mutation between this read and the following rename remains an
/// accepted residual cross-process risk; callers must not claim cross-process
/// compare-and-swap semantics.
pub(crate) fn canonical_bytes_unchanged(path: &Path, expected: &[u8]) -> io::Result<bool> {
    #[cfg(test)]
    {
        let barrier = TEST_REPLACEMENT_BARRIERS
            .lock()
            .ok()
            .and_then(|mut barriers| {
                barriers
                    .iter()
                    .position(|barrier| barrier.path == path)
                    .map(|index| barriers.remove(index))
            });
        if let Some(barrier) = barrier {
            let _ = barrier.started.send(());
            let _ = barrier.release.recv();
        }
    }

    fs::read(path).map(|current| current == expected)
}

#[cfg(test)]
pub(crate) fn install_replacement_barrier(
    path: PathBuf,
) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    TEST_REPLACEMENT_BARRIERS
        .lock()
        .expect("replacement barrier lock")
        .push(TestReplacementBarrier {
            path,
            started: started_tx,
            release: release_rx,
        });
    (started_rx, release_tx)
}

pub(crate) fn existing_file_permissions(path: &Path) -> io::Result<Option<fs::Permissions>> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata.permissions())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn create_new_file_with_permissions(
    path: &Path,
    permissions: Option<&fs::Permissions>,
) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    if let Some(permissions) = permissions {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(permissions.mode());
    }

    let file = options.open(path)?;
    if let Some(permissions) = permissions {
        if let Err(error) = file.set_permissions(permissions.clone()) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(error);
        }
    }
    Ok(file)
}

#[cfg(test)]
fn install_quarantine_validation_barrier(
    path: PathBuf,
) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *TEST_QUARANTINE_VALIDATION_BARRIER
        .lock()
        .expect("quarantine validation barrier lock") = Some(TestQuarantineValidationBarrier {
        path,
        started: started_tx,
        release: release_rx,
    });
    (started_rx, release_tx)
}

fn remove_failed_quarantine(path: &Path) {
    if fs::remove_file(path).is_ok() {
        let _ = sync_parent_directory(path);
    }
}

/// Preserve an invalid canonical file under a uniquely named sibling before
/// an explicit start-new operation. A same-directory hard link names the exact
/// same file object, so no sensitive bytes are ever copied into a newly created
/// file with parent-inherited access controls. This also preserves the inode,
/// permissions, and (on Windows) security descriptor of the canonical file.
///
/// If the filesystem cannot create hard links, recovery fails closed and the
/// canonical path is untouched. The app serializes every internal recovery and
/// single-instance operation. The final byte checks narrow, but cannot
/// portably eliminate, arbitrary external mutation before canonical rename.
pub(crate) fn quarantine_invalid_hard_link(
    path: &Path,
    expected_contents: &[u8],
) -> io::Result<PathBuf> {
    #[cfg(test)]
    if TEST_QUARANTINE_FAILURES.lock().is_ok_and(|mut targets| {
        targets
            .iter()
            .position(|target| target == path)
            .map(|index| targets.remove(index))
            .is_some()
    }) {
        return Err(io::Error::other("injected quarantine failure"));
    }

    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "storage path has no parent"))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "local-storage".into());
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    for _ in 0..100 {
        let id = QUARANTINE_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let quarantine_path = parent.join(format!(
            "{name}.invalid-{timestamp_ms}-{}-{id}",
            std::process::id()
        ));
        match fs::hard_link(path, &quarantine_path) {
            Ok(()) => {
                #[cfg(test)]
                {
                    let barrier =
                        TEST_QUARANTINE_VALIDATION_BARRIER
                            .lock()
                            .ok()
                            .and_then(|mut slot| {
                                if slot.as_ref().is_some_and(|barrier| barrier.path == path) {
                                    slot.take()
                                } else {
                                    None
                                }
                            });
                    if let Some(barrier) = barrier {
                        let _ = barrier.started.send(());
                        let _ = barrier.release.recv();
                    }
                }

                let validation = fs::read(&quarantine_path).and_then(|contents| {
                    if contents == expected_contents {
                        Ok(())
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "canonical bytes changed while quarantine was being preserved",
                        ))
                    }
                });
                if let Err(error) = validation {
                    remove_failed_quarantine(&quarantine_path);
                    return Err(error);
                }
                if let Err(error) = sync_parent_directory(&quarantine_path) {
                    remove_failed_quarantine(&quarantine_path);
                    return Err(error);
                }
                return Ok(quarantine_path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a quarantine hard link",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "unfocus-{label}-test-{}-{}",
            std::process::id(),
            QUARANTINE_FILE_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn quarantine_is_a_byte_identical_unique_sibling() {
        let root = test_root("storage-recovery");
        fs::create_dir(&root).expect("test directory");
        let path = root.join("activity-history.json");
        let bytes = b"{not valid json}\0\xff";
        fs::write(&path, bytes).expect("canonical bytes");

        let first = quarantine_invalid_hard_link(&path, bytes).expect("first quarantine");
        let second = quarantine_invalid_hard_link(&path, bytes).expect("second quarantine");

        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(root.as_path()));
        assert_eq!(fs::read(first).expect("first bytes"), bytes);
        assert_eq!(fs::read(second).expect("second bytes"), bytes);
        assert_eq!(fs::read(path).expect("canonical unchanged"), bytes);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn quarantine_preserves_the_canonical_inode_and_mode() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let root = test_root("storage-hard-link");
        fs::create_dir(&root).expect("test directory");
        let path = root.join("break-events.json");
        fs::write(&path, b"invalid").expect("canonical bytes");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("restrict canonical permissions");
        let canonical = fs::metadata(&path).expect("canonical metadata");

        let quarantine =
            quarantine_invalid_hard_link(&path, b"invalid").expect("quarantine hard link");
        let preserved = fs::metadata(&quarantine).expect("quarantine metadata");

        assert_eq!(preserved.dev(), canonical.dev());
        assert_eq!(preserved.ino(), canonical.ino());
        assert_eq!(preserved.permissions().mode() & 0o7777, 0o600);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn quarantine_stays_old_and_independent_after_canonical_replacement() {
        let root = test_root("storage-replacement-independence");
        fs::create_dir(&root).expect("test directory");
        let path = root.join("reminder-settings.json");
        let old_bytes = b"invalid old settings";
        fs::write(&path, old_bytes).expect("canonical bytes");
        let quarantine =
            quarantine_invalid_hard_link(&path, old_bytes).expect("quarantine hard link");
        let replacement = root.join("replacement.tmp");
        fs::write(&replacement, b"new settings").expect("replacement bytes");

        replace_file_atomically(&replacement, &path).expect("canonical replacement");
        assert_eq!(fs::read(&quarantine).expect("preserved bytes"), old_bytes);
        assert_eq!(fs::read(&path).expect("new canonical"), b"new settings");

        fs::write(&path, b"later canonical update").expect("later canonical write");
        assert_eq!(
            fs::read(&quarantine).expect("still preserved bytes"),
            old_bytes
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn canonical_replacement_detaches_the_quarantine_inode() {
        use std::os::unix::fs::MetadataExt;

        let root = test_root("storage-replacement-inode");
        fs::create_dir(&root).expect("test directory");
        let path = root.join("activity-history.json");
        fs::write(&path, b"invalid old history").expect("canonical bytes");
        let quarantine = quarantine_invalid_hard_link(&path, b"invalid old history")
            .expect("quarantine hard link");
        let old_inode = fs::metadata(&quarantine).expect("old metadata").ino();
        let replacement = root.join("replacement.tmp");
        fs::write(&replacement, b"new history").expect("replacement bytes");

        replace_file_atomically(&replacement, &path).expect("canonical replacement");

        assert_eq!(
            fs::metadata(&quarantine).expect("quarantine").ino(),
            old_inode
        );
        assert_ne!(fs::metadata(&path).expect("canonical").ino(), old_inode);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_in_place_mutation_fails_quarantine_validation() {
        use std::time::Duration;

        let root = test_root("storage-concurrent-validation");
        fs::create_dir(&root).expect("test directory");
        let path = root.join("break-events.json");
        let original = b"invalid old ledger";
        let external = b"external mutation";
        fs::write(&path, original).expect("canonical bytes");
        let (started, release) = install_quarantine_validation_barrier(path.clone());
        let quarantine_path = path.clone();
        let quarantine =
            std::thread::spawn(move || quarantine_invalid_hard_link(&quarantine_path, original));
        started
            .recv_timeout(Duration::from_secs(1))
            .expect("hard link reaches validation barrier");

        fs::write(&path, external).expect("external canonical mutation");
        release.send(()).expect("release quarantine validation");
        let error = quarantine
            .join()
            .expect("quarantine thread")
            .expect_err("changed bytes must fail validation");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&path).expect("external bytes remain"), external);
        assert_eq!(
            fs::read_dir(&root)
                .expect("siblings")
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("break-events.json.invalid-"))
                .count(),
            0
        );
        let _ = fs::remove_dir_all(root);
    }
}
