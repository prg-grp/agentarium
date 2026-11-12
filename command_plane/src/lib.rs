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

/// The vsock port the supervisor accepts forwarded ports on, each connection starting
/// with a `Forward`. Below 1024 for the same reason as `PORT`.
pub const FORWARD_PORT: u32 = 702;

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

/// The guest's side of a vsock connection: listening and accepting. Used by the guest
/// supervisor (command plane) and the guest observer (data plane).
#[cfg(target_os = "linux")]
pub mod vsock {
    use std::fs::File;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    /// Listens on vsock `port` for the host. Ports up to 1023 need root.
    pub fn listen(port: u32) -> io::Result<OwnedFd> {
        let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut address: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
        address.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        address.svm_port = port;
        address.svm_cid = libc::VMADDR_CID_ANY;
        let bound = unsafe {
            libc::bind(
                socket.as_raw_fd(),
                (&raw const address).cast(),
                size_of::<libc::sockaddr_vm>() as libc::socklen_t,
            )
        };
        // Room for the burst of connections a browser opens to a forwarded port.
        if bound < 0 || unsafe { libc::listen(socket.as_raw_fd(), libc::SOMAXCONN) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(socket)
    }

    /// Accepts the next connection; returns it with the peer's vsock CID and port.
    pub fn accept(listener: &OwnedFd) -> io::Result<(File, u32, u32)> {
        let mut address: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
        let mut length = size_of::<libc::sockaddr_vm>() as libc::socklen_t;
        let fd = unsafe {
            libc::accept4(
                listener.as_raw_fd(),
                (&raw mut address).cast(),
                &mut length,
                libc::SOCK_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((
            unsafe { File::from_raw_fd(fd) },
            address.svm_cid,
            address.svm_port,
        ))
    }
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
