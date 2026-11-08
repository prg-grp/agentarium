//! The data plane: protobuf messages from the guest observer to the app over virtio-vsock.
//! The protocol is in `proto/data_plane.proto`; this crate holds the generated types. The
//! frames are the command plane's (`read_message`, `write_message`).

use std::time::Duration;

#[cfg(target_os = "linux")]
pub use command_plane::vsock;
pub use command_plane::{MAX_MESSAGE, Message, read_message, write_message};

include!(concat!(env!("OUT_DIR"), "/agent_toolbox.data_plane.v1.rs"));

/// The vsock port the observer listens on. Only root can listen on ports up to 1023 in a
/// Linux guest, so the agent user can't pose as the observer.
pub const PORT: u32 = 701;

/// Sent in `Hello`; the app ignores an observer that speaks another version.
pub const PROTOCOL_VERSION: u32 = 1;

/// How often the observer sends `Health`.
pub const HEALTH_INTERVAL: Duration = Duration::from_secs(2);
