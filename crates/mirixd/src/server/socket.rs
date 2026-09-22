//! Unix socket ownership and stale-socket recovery.
use anyhow::{Context, Result, bail};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(super) struct SocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl SocketGuard {
    fn new(path: PathBuf) -> io::Result<Self> {
        let metadata = std::fs::symlink_metadata(&path)?;
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub(super) fn bind_socket(path: &Path) -> Result<(UnixListener, SocketGuard)> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create socket directory {}", parent.display()))?;
    }

    let listener = match UnixListener::bind(path) {
        Ok(listener) => listener,
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => match UnixStream::connect(path) {
            Ok(_) => bail!("mirixd is already running at {}", path.display()),
            Err(connect_err)
                if matches!(
                    connect_err.kind(),
                    io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                ) =>
            {
                let metadata = std::fs::symlink_metadata(path)
                    .with_context(|| format!("inspect stale socket {}", path.display()))?;
                if !metadata.file_type().is_socket() {
                    bail!("refusing to replace non-socket path {}", path.display());
                }
                std::fs::remove_file(path)
                    .with_context(|| format!("remove stale socket {}", path.display()))?;
                UnixListener::bind(path).with_context(|| {
                    format!("bind {} after stale socket cleanup", path.display())
                })?
            }
            Err(connect_err) => {
                return Err(connect_err)
                    .with_context(|| format!("check existing socket {}", path.display()));
            }
        },
        Err(err) => return Err(err).with_context(|| format!("bind {}", path.display())),
    };
    let guard = SocketGuard::new(path.to_path_buf())
        .with_context(|| format!("track socket {}", path.display()))?;
    Ok((listener, guard))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mirixd-test-{}-{id}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn socket(&self) -> PathBuf {
            self.0.join("mirix.sock")
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn rejects_an_active_socket_without_removing_it() {
        let dir = TestDir::new();
        let socket = dir.socket();
        let (_listener, guard) = bind_socket(&socket).unwrap();

        let error = bind_socket(&socket).unwrap_err();

        assert!(error.to_string().contains("already running"));
        assert!(socket.exists());
        drop(guard);
        assert!(!socket.exists());
    }

    #[test]
    fn replaces_a_stale_socket() {
        let dir = TestDir::new();
        let socket = dir.socket();
        drop(UnixListener::bind(&socket).unwrap());

        let (_listener, guard) = bind_socket(&socket).unwrap();

        assert!(socket.exists());
        drop(guard);
        assert!(!socket.exists());
    }

    #[test]
    fn guard_does_not_remove_a_replacement_socket() {
        let dir = TestDir::new();
        let socket = dir.socket();
        let moved_socket = dir.0.join("old.sock");
        let (_listener, guard) = bind_socket(&socket).unwrap();
        std::fs::rename(&socket, &moved_socket).unwrap();
        let replacement = UnixListener::bind(&socket).unwrap();

        drop(guard);

        assert!(socket.exists());
        drop(replacement);
    }

    #[test]
    fn refuses_to_replace_a_non_socket_path() {
        let dir = TestDir::new();
        let socket = dir.socket();
        std::fs::write(&socket, "not a socket").unwrap();

        let error = bind_socket(&socket).unwrap_err();

        assert!(error.to_string().contains("non-socket"));
        assert_eq!(std::fs::read_to_string(socket).unwrap(), "not a socket");
    }
}
