//! Daemon messages and shared versioned v1 JSON transport.
mod input_methods;
mod messages;
pub use input_methods::{InputMethodCatalog, InstalledInputMethod};
mod runtime;

pub use messages::{Request, Response, UiPreference};
pub use mirix_engine_protocol::{
    FRAME_HEADER_SIZE, FrameError, MAX_FRAME_SIZE, PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
    decode_frame, encode_frame,
};
pub use mirix_engine_protocol::{TransportError, read_message, write_message};
pub use runtime::default_socket_path;
