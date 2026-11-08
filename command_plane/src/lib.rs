//! The command plane: protobuf messages between the app and the guest supervisor over
//! virtio-vsock. The protocol is in `proto/command_plane.proto`; this crate holds the
//! generated types and the framing both sides use.

use std::io::{self, Read, Write};
use std::time::Duration;

pub use prost::Message;

include!(concat!(
    env!("OUT_DIR"),
    "/agent_toolbox.command_plane.v1.rs"
));

/// The vsock port the supervisor listens on. Only root can listen on ports up to 1023
/// in a Linux guest, so the agent user can't pose as the supervisor.
pub const PORT: u32 = 700;

/// Sent in `Hello`; the app refuses a supervisor that speaks another version.
pub const PROTOCOL_VERSION: u32 = 1;

/// The tag of the virtio-fs device that holds an instance's shared folders.
pub const SHARE_TAG: &str = "agentarium-shares";

/// How often the supervisor reports `Status` in agent mode.
pub const STATUS_INTERVAL: Duration = Duration::from_secs(2);

/// The largest encoded message either side sends or accepts.
pub const MAX_MESSAGE: usize = 1 << 20;

/// Writes `message` as a big-endian u32 length followed by its encoding.
pub fn write_message(writer: &mut impl Write, message: &impl Message) -> io::Result<()> {
    let len = message.encoded_len();
    if len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("the message is {len} bytes, more than {MAX_MESSAGE}"),
        ));
    }
    let mut frame = Vec::with_capacity(4 + len);
    frame.extend_from_slice(&(len as u32).to_be_bytes());
    message
        .encode(&mut frame)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    writer.write_all(&frame)?;
    writer.flush()
}

/// Reads one message written by `write_message`. A closed connection is `UnexpectedEof`.
pub fn read_message<M: Message + Default>(reader: &mut impl Read) -> io::Result<M> {
    let mut len = [0; 4];
    reader.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the message is {len} bytes, more than {MAX_MESSAGE}"),
        ));
    }
    let mut buf = vec![0; len];
    reader.read_exact(&mut buf)?;
    M::decode(buf.as_slice()).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
