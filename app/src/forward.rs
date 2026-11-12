//! The forwarded ports of one boot: listeners on the Mac whose connections the guest
//! supervisor relays to ports on the guest's loopback.
//!
//! Each accepted connection is handed to the controller, which opens a vsock connection to
//! the supervisor's `FORWARD_PORT`. The app names the guest port in a `Forward`, then both
//! sides copy the bytes. The guest's network and its policy aren't involved.

use std::fs::File;
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, TcpListener as StdTcpListener, TcpStream as StdTcpStream};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::time::Duration;

use command_plane::{Forward, write_message};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::runtime::{Handle, Runtime};

use crate::db::PortForward;

/// Runs a boot's listeners until dropped.
pub struct Forwards {
    runtime: Option<Runtime>,
}

/// How a forward of the boot turned out.
pub struct Forwarded {
    pub guest_port: u16,
    /// The port listened on; for a failed forward the fixed port asked for, if any.
    pub host_port: Option<u16>,
    /// Why the Mac doesn't listen, briefly; `None` while it does.
    pub error: Option<String>,
}

impl Forwards {
    /// Listens for each forward. A port that can't be opened, such as one in use, is
    /// reported in its `Forwarded` and leaves the others working. `accepted` gets each
    /// connection with its guest port.
    pub fn start(
        forwards: &[PortForward],
        accepted: impl Fn(StdTcpStream, u16) + Clone + Send + 'static,
    ) -> io::Result<(Forwards, Vec<Forwarded>)> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("vm-forward")
            .enable_all()
            .build()?;
        let opened = forwards
            .iter()
            .map(|forward| {
                let address = if forward.all_interfaces {
                    Ipv4Addr::UNSPECIFIED
                } else {
                    Ipv4Addr::LOCALHOST
                };
                // Port 0 has the system pick a free one.
                let listener = StdTcpListener::bind((address, forward.host_port.unwrap_or(0)))
                    .and_then(|listener| {
                        listener.set_nonblocking(true)?;
                        let port = listener.local_addr()?.port();
                        let _context = runtime.enter();
                        Ok((TcpListener::from_std(listener)?, port))
                    });
                match listener {
                    Ok((listener, port)) => {
                        runtime.spawn(accept(listener, forward.guest_port, accepted.clone()));
                        Forwarded {
                            guest_port: forward.guest_port,
                            host_port: Some(port),
                            error: None,
                        }
                    }
                    Err(error) => {
                        eprintln!(
                            "Couldn't forward guest port {} on {address}: {error}",
                            forward.guest_port
                        );
                        Forwarded {
                            guest_port: forward.guest_port,
                            host_port: forward.host_port,
                            error: Some(if error.kind() == ErrorKind::AddrInUse {
                                "in use".into()
                            } else {
                                "unavailable".into()
                            }),
                        }
                    }
                }
            })
            .collect();
        Ok((
            Forwards {
                runtime: Some(runtime),
            },
            opened,
        ))
    }

    /// Where `relay` runs.
    pub fn handle(&self) -> Option<Handle> {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.handle().clone())
    }
}

impl Drop for Forwards {
    fn drop(&mut self) {
        // Closes the listeners and connections without waiting, like the VM's network.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

async fn accept(
    listener: TcpListener,
    guest_port: u16,
    accepted: impl Fn(StdTcpStream, u16) + Send + 'static,
) {
    loop {
        match listener.accept().await {
            Ok((client, _)) => match client.into_std() {
                Ok(client) => accepted(client, guest_port),
                Err(error) => eprintln!("Couldn't take a forwarded connection: {error}"),
            },
            // E.g. out of file descriptors; waiting gives connections time to close.
            Err(error) => {
                eprintln!("Couldn't accept a connection for guest port {guest_port}: {error}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// Connects a forwarded connection to the supervisor's vsock connection `guest`: names the
/// guest port, then copies both directions until both are done.
pub async fn relay(client: StdTcpStream, guest: File, guest_port: u16) {
    let mut guest = StdUnixStream::from(OwnedFd::from(guest));
    let guest = write_message(
        &mut guest,
        &Forward {
            port: guest_port.into(),
        },
    )
    .and_then(|()| guest.set_nonblocking(true))
    .and_then(|()| UnixStream::from_std(guest));
    let (Ok(mut guest), Ok(mut client)) = (guest, TcpStream::from_std(client)) else {
        return;
    };
    let _ = tokio::io::copy_bidirectional(&mut client, &mut guest).await;
}
