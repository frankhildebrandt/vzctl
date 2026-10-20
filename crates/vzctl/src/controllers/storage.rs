//! Private credentials, accessed relative to pinned directory descriptors.
use super::{ControllerError, ControllerResult, SecretStore};
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

const MAX_SECRET_BYTES: u64 = 1024 * 1024;

fn failure() -> ControllerError {
    ControllerError::new(
        "SecretStorage",
        "private credential state is unavailable or unsafe",
    )
}

pub(super) fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(super) fn read_private(path: &Path) -> ControllerResult<Vec<u8>> {
    let parent = path.parent().ok_or_else(failure)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(failure)?;
    PrivateDirectory::open(parent)?
        .read(name)?
        .ok_or_else(failure)
}

/// The controller holds its project lock across read/reconcile/write.
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> ControllerResult<()> {
    let parent = path.parent().ok_or_else(failure)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(failure)?;
    let directory = PrivateDirectory::open(parent)?;
    // Reject an unsafe existing target. The rename itself never follows symlinks.
    directory.read(name)?;
    directory.write(name, bytes)
}

pub(super) struct Lock {
    _file: File,
}
impl Lock {
    pub(super) fn acquire(path: &Path) -> ControllerResult<Self> {
        let directory = PrivateDirectory::open(path)?;
        let file = directory.open_file(".lock", libc::O_RDWR | libc::O_CREAT)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(ControllerError::new(
                "LeaseHeld",
                "another controller reconciliation holds the project lock",
            ));
        }
        Ok(Self { _file: file })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // CLOEXEC closes the child's descriptor only at exec. Until then a
        // concurrent fork can keep this open description alive after our close.
        // This owner has finished the critical section, so release explicitly.
        unsafe {
            libc::flock(self._file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

pub(crate) fn random_secret() -> ControllerResult<String> {
    let mut bytes = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| failure())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) struct PrivateDirectory {
    directory: File,
}

impl PrivateDirectory {
    pub(crate) fn open(path: &Path) -> ControllerResult<Self> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_err(|_| failure())?.join(path)
        };
        // macOS exposes these system-owned aliases; all user-controlled components
        // below them still use openat(O_NOFOLLOW), never check-then-open.
        #[cfg(target_os = "macos")]
        let path = if path.starts_with("/tmp") || path.starts_with("/var") {
            Path::new("/private").join(path.strip_prefix("/").map_err(|_| failure())?)
        } else {
            path
        };
        let mut directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")
            .map_err(|_| failure())?;
        let mut count = 0;
        for part in path.components() {
            let Component::Normal(name) = part else {
                if matches!(part, Component::RootDir) {
                    continue;
                }
                return Err(failure());
            };
            let name = CString::new(name.as_bytes()).map_err(|_| failure())?;
            let parent = directory.as_raw_fd();
            let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            let mut fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
            if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                let created = unsafe { libc::mkdirat(parent, name.as_ptr(), 0o700) };
                if created != 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(failure());
                }
                fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
            }
            if fd < 0 {
                return Err(failure());
            }
            directory = unsafe { File::from_raw_fd(fd) };
            count += 1;
        }
        if count == 0
            || directory.metadata().map_err(|_| failure())?.uid() != unsafe { libc::geteuid() }
        {
            return Err(failure());
        }
        if unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) } != 0 {
            return Err(failure());
        }
        Ok(Self { directory })
    }

    fn name(name: &str) -> ControllerResult<CString> {
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err(failure());
        }
        CString::new(name).map_err(|_| failure())
    }

    fn open_file(&self, name: &str, flags: i32) -> ControllerResult<File> {
        let name = Self::name(name)?;
        let mut attempt = 0;
        let fd = loop {
            let fd = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                    0o600,
                )
            };
            if fd >= 0 {
                break fd;
            }
            let error = std::io::Error::last_os_error();
            // Concurrent first-lock creation has produced transient ENOENT on
            // macOS in tests; its cause is not established. Retry only creates,
            // bounded and against the same pinned directory with all checks.
            if attempt >= 8
                || !(error.kind() == std::io::ErrorKind::Interrupted
                    || (flags & libc::O_CREAT != 0 && error.kind() == std::io::ErrorKind::NotFound))
            {
                return Err(failure());
            }
            attempt += 1;
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata().map_err(|_| failure())?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(failure());
        }
        Ok(file)
    }

    pub(crate) fn locked<T>(
        &self,
        operation: impl FnOnce() -> ControllerResult<T>,
    ) -> ControllerResult<T> {
        let lock = self.open_file(".lock", libc::O_RDWR | libc::O_CREAT)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(failure());
        }
        // Share the same explicit-unlock guard on success, error and unwind.
        let _guard = Lock { _file: lock };
        operation()
    }

    pub(crate) fn read(&self, name: &str) -> ControllerResult<Option<Vec<u8>>> {
        let name_c = Self::name(name)?;
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name_c.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(failure())
            };
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata().map_err(|_| failure())?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
            || metadata.len() > MAX_SECRET_BYTES
        {
            return Err(failure());
        }
        let mut bytes = Vec::new();
        file.take(MAX_SECRET_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failure())?;
        if bytes.len() as u64 > MAX_SECRET_BYTES {
            return Err(failure());
        }
        Ok(Some(bytes))
    }

    /// Caller holds this directory's lock. Rename swaps the complete private file
    /// atomically; it never follows the destination, including during replacement.
    pub(crate) fn write(&self, name: &str, bytes: &[u8]) -> ControllerResult<()> {
        let destination = Self::name(name)?;
        let temporary = format!(".new-{}", random_secret()?);
        let temp_c = Self::name(&temporary)?;
        let mut file = self.open_file(&temporary, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
        let result = (|| {
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| failure())?;
            if unsafe {
                libc::renameat(
                    self.directory.as_raw_fd(),
                    temp_c.as_ptr(),
                    self.directory.as_raw_fd(),
                    destination.as_ptr(),
                )
            } != 0
            {
                return Err(failure());
            }
            self.directory.sync_all().map_err(|_| failure())?;
            Ok(())
        })();
        if result.is_err() {
            unsafe {
                libc::unlinkat(self.directory.as_raw_fd(), temp_c.as_ptr(), 0);
            }
        }
        result
    }
}

pub(crate) struct FileSecretStore {
    directory: PrivateDirectory,
}

impl FileSecretStore {
    pub(crate) fn new(path: PathBuf) -> ControllerResult<Self> {
        Ok(Self {
            directory: PrivateDirectory::open(&path)?,
        })
    }

    pub(crate) fn get_or_import(
        &self,
        key: &str,
        existing: Option<&str>,
    ) -> ControllerResult<String> {
        let filename = format!("{:x}.secret", Sha256::digest(key.as_bytes()));
        self.directory.locked(|| {
            if let Some(bytes) = self.directory.read(&filename)? {
                let secret = String::from_utf8(bytes).map_err(|_| failure())?;
                if secret.trim().is_empty() {
                    return Err(failure());
                }
                return Ok(secret);
            }
            let secret = match existing {
                Some(value) if !value.trim().is_empty() => value.to_string(),
                Some(_) => return Err(failure()),
                None => random_secret()?,
            };
            self.directory.write(&filename, secret.as_bytes())?;
            Ok(secret)
        })
    }
}

impl SecretStore for FileSecretStore {
    fn get_or_create(&self, key: &str) -> ControllerResult<String> {
        self.get_or_import(key, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_get_or_create_is_stable_private_and_fails_closed() {
        let path =
            std::env::temp_dir().join(format!("vzctl-secret-test-{}", random_secret().unwrap()));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    FileSecretStore::new(path)
                        .unwrap()
                        .get_or_create("client/web")
                        .unwrap()
                })
            })
            .collect();
        let values: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert!(values.iter().all(|value| value == &values[0]));
        assert_eq!(values[0].len(), 64);
        let file = path.join(format!("{:x}.secret", Sha256::digest(b"client/web")));
        assert_eq!(file.metadata().unwrap().mode() & 0o777, 0o600);
        std::fs::write(&file, b"").unwrap();
        assert!(FileSecretStore::new(path.clone())
            .unwrap()
            .get_or_create("client/web")
            .is_err());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn refuses_directory_symlink_without_modifying_its_target() {
        let path =
            std::env::temp_dir().join(format!("vzctl-secret-link-{}", random_secret().unwrap()));
        std::fs::create_dir_all(path.join("target")).unwrap();
        std::os::unix::fs::symlink(path.join("target"), path.join("alias")).unwrap();
        assert!(FileSecretStore::new(path.join("alias")).is_err());
        assert!(std::fs::read_dir(path.join("target"))
            .unwrap()
            .next()
            .is_none());
        std::fs::remove_dir_all(path).unwrap();
    }

    /// The child executes only async-signal-safe syscalls after fork. It keeps
    /// inherited descriptors open until the parent explicitly lets it exit.
    fn child_holding_inherited_descriptors() -> (libc::pid_t, libc::c_int) {
        let mut control = [-1; 2];
        assert_eq!(unsafe { libc::pipe(control.as_mut_ptr()) }, 0);
        let child = unsafe { libc::fork() };
        if child == 0 {
            unsafe {
                libc::close(control[1]);
                let mut byte = 0u8;
                libc::read(control[0], (&mut byte as *mut u8).cast(), 1);
                libc::_exit(0);
            }
        }
        unsafe {
            libc::close(control[0]);
        }
        if child < 0 {
            unsafe {
                libc::close(control[1]);
            }
        }
        assert!(child > 0);
        (child, control[1])
    }

    fn release_child(child: (libc::pid_t, libc::c_int)) {
        unsafe {
            let byte = 1u8;
            libc::write(child.1, (&byte as *const u8).cast(), 1);
            libc::close(child.1);
            let mut status = 0;
            while libc::waitpid(child.0, &mut status, 0) < 0 {
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    break;
                }
            }
        }
    }

    #[test]
    fn owner_drop_unlocks_even_while_a_fork_child_keeps_the_descriptor_open() {
        for private_helper in [false, true] {
            let path =
                std::env::temp_dir().join(format!("vzctl-lock-fork-{}", random_secret().unwrap()));
            let child = if private_helper {
                PrivateDirectory::open(&path)
                    .unwrap()
                    .locked(|| Ok(child_holding_inherited_descriptors()))
                    .unwrap()
            } else {
                let first = Lock::acquire(&path).unwrap();
                let child = child_holding_inherited_descriptors();
                drop(first);
                child
            };
            // No wait or retry: child still holds its inherited lock descriptor.
            let reacquired = Lock::acquire(&path);
            release_child(child);
            assert!(
                reacquired.is_ok(),
                "released owner must not leave a lock in an unrelated child: {private_helper}"
            );
            drop(reacquired);
            std::fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn project_lock_reports_contention_without_waiting() {
        let path =
            std::env::temp_dir().join(format!("vzctl-lock-test-{}", random_secret().unwrap()));
        let first = Lock::acquire(&path).unwrap();
        let error = match Lock::acquire(&path) {
            Ok(_) => panic!("concurrent project lock succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.code, "LeaseHeld");
        drop(first);
        drop(Lock::acquire(&path).unwrap());
        std::fs::remove_dir_all(path).unwrap();
    }
}
