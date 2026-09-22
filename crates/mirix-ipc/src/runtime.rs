const SOCKET_NAME: &str = "mirix.sock";

/// `$XDG_RUNTIME_DIR/mirix/mirix.sock`, falling back to `/tmp/mirix/mirix.sock`.
pub fn default_socket_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join("mirix").join(SOCKET_NAME)
}
