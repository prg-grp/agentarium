//! Guest networking for one VM: a userspace TCP/IP stack behind its virtio NIC.
//!
//! Virtualization.framework passes the guest's Ethernet frames through a datagram
//! socketpair. The stack (netstack-smoltcp) terminates the guest's TCP connections and
//! UDP flows, and each one is opened again as an ordinary socket on the Mac, so every
//! connection passes the VM's `Policy` first. DNS (UDP port 53, to any address) always goes
//! to the Mac's nameserver, so the policy sees every lookup and answer. Each VM gets its
//! own runtime and threads, so one VM's traffic can't slow down another's.
//!
//! The guest is configured statically (base rootfs): 192.168.127.2/24, gateway and DNS
//! at 192.168.127.1, IPv4 only.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram as StdUnixDatagram;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use etherparse::{
    ArpEthIpv4Packet, ArpOperation, ArpPacket, EtherType, Ethernet2Header, Ethernet2Slice,
};
use futures::{SinkExt, StreamExt};
use netstack_smoltcp::udp::UdpMsg;
use netstack_smoltcp::{StackBuilder, TcpListener};
use tokio::net::{TcpStream, UdpSocket, UnixDatagram};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::time::{sleep, timeout};

use crate::policy::Policy;

/// MTU of the guest NIC and the stack. 1500 is Virtualization.framework's default.
pub const MTU: usize = 1500;
const GATEWAY_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 127, 1);
const GATEWAY_MAC: [u8; 6] = [0x02, 0, 0, 0, 0, 0x01];
const DNS_PORT: u16 = 53;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const UDP_IDLE: Duration = Duration::from_secs(60);
/// DNS is one request and one answer, so its host sockets are released quickly.
const DNS_IDLE: Duration = Duration::from_secs(5);

/// Runs a VM's network until dropped.
pub struct Network {
    runtime: Option<Runtime>,
}

impl Network {
    /// Starts the network under `policy` and returns it with the guest's end of the
    /// socketpair, which becomes the VM's network device.
    pub fn start(policy: Arc<Policy>) -> io::Result<(Network, StdUnixDatagram)> {
        let (guest, host) = StdUnixDatagram::pair()?;
        for socket in [&guest, &host] {
            set_buffers(socket)?;
        }
        host.set_nonblocking(true)?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("vm-net")
            .enable_all()
            .build()?;
        let host = {
            let _context = runtime.enter();
            UnixDatagram::from_std(host)?
        };
        runtime.spawn(async move {
            if let Err(error) = run(host, upstream_dns(), policy).await {
                eprintln!("The VM network stopped: {error}");
            }
        });
        Ok((
            Network {
                runtime: Some(runtime),
            },
            guest,
        ))
    }
}

impl Drop for Network {
    fn drop(&mut self) {
        // Doesn't wait for the tasks, so stopping a VM never blocks the UI thread.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// Moves frames between the guest and the stack, and starts the TCP and UDP relays.
async fn run(host: UnixDatagram, dns: SocketAddr, policy: Arc<Policy>) -> io::Result<()> {
    let (stack, runner, udp, tcp) = StackBuilder::default()
        .enable_tcp(true)
        .enable_udp(true)
        .mtu(MTU)
        .build()?;
    tokio::spawn(runner.expect("TCP is enabled"));
    tokio::spawn(relay_tcp(tcp.expect("TCP is enabled"), policy.clone()));
    tokio::spawn(relay_udp(udp.expect("UDP is enabled"), dns, policy));

    let host = Arc::new(host);
    let guest_mac = Arc::new(OnceLock::new());
    let (mut to_stack, mut from_stack) = stack.split();

    // Stack to guest: wrap each IP packet in an Ethernet frame.
    let (out_host, out_mac) = (host.clone(), guest_mac.clone());
    tokio::spawn(async move {
        let mut frame = Vec::with_capacity(Ethernet2Header::LEN + MTU);
        while let Some(packet) = from_stack.next().await {
            // Before the guest's first frame its MAC is unknown; it always ARPs first.
            let (Ok(packet), Some(&mac)) = (packet, out_mac.get()) else {
                continue;
            };
            let header = Ethernet2Header {
                source: GATEWAY_MAC,
                destination: mac,
                ether_type: EtherType::IPV4,
            };
            frame.clear();
            frame.extend_from_slice(&header.to_bytes());
            frame.extend_from_slice(&packet);
            // A frame the guest can't take right now is dropped; TCP resends it.
            let _ = out_host.send(&frame).await;
        }
    });

    // Guest to stack: answer ARP for the gateway, pass IPv4 on, drop the rest.
    let mut buffer = vec![0; 65536];
    loop {
        let len = host.recv(&mut buffer).await?;
        let Ok(frame) = Ethernet2Slice::from_slice_without_fcs(&buffer[..len]) else {
            continue;
        };
        guest_mac.get_or_init(|| frame.source());
        match frame.ether_type() {
            EtherType::IPV4 => to_stack.send(frame.payload_slice().to_vec()).await?,
            EtherType::ARP => {
                if let Some(reply) = arp_reply(frame.payload_slice()) {
                    host.send(&reply).await?;
                }
            }
            _ => {}
        }
    }
}

/// The reply frame to an ARP request for the gateway's address.
fn arp_reply(payload: &[u8]) -> Option<Vec<u8>> {
    let request = ArpPacket::from_slice(payload).ok()?.try_eth_ipv4().ok()?;
    if request.operation != ArpOperation::REQUEST || request.target_ipv4_addr() != GATEWAY_IP {
        return None;
    }
    let reply = ArpEthIpv4Packet {
        operation: ArpOperation::REPLY,
        sender_mac: GATEWAY_MAC,
        sender_ipv4: GATEWAY_IP.octets(),
        target_mac: request.sender_mac,
        target_ipv4: request.sender_ipv4,
    };
    let header = Ethernet2Header {
        source: GATEWAY_MAC,
        destination: request.sender_mac,
        ether_type: EtherType::ARP,
    };
    Some([&header.to_bytes()[..], &reply.to_bytes()].concat())
}

/// Opens each guest TCP connection the policy allows on the Mac and copies data both ways.
/// A blocked connection is closed right away.
async fn relay_tcp(mut listener: TcpListener, policy: Arc<Policy>) {
    while let Some((mut guest, _, destination)) = listener.next().await {
        if !policy.allows(destination.ip()) {
            eprintln!("Blocked a TCP connection to {destination}");
            continue;
        }
        tokio::spawn(async move {
            let mut remote = match timeout(CONNECT_TIMEOUT, TcpStream::connect(destination)).await {
                Ok(Ok(remote)) => remote,
                Ok(Err(error)) => return eprintln!("Couldn't connect to {destination}: {error}"),
                Err(_) => return eprintln!("Connecting to {destination} timed out"),
            };
            let _ = remote.set_nodelay(true);
            let _ = tokio::io::copy_bidirectional(&mut guest, &mut remote).await;
        });
    }
}

/// Sends each guest UDP flow the policy allows from its own socket on the Mac. DNS queries,
/// to the gateway or any other address, go to the Mac's nameserver if the policy allows
/// their name, and are refused otherwise.
async fn relay_udp(socket: netstack_smoltcp::UdpSocket, dns: SocketAddr, policy: Arc<Policy>) {
    let (mut from_guest, mut to_guest) = socket.split();
    let (reply_tx, mut reply_rx) = mpsc::channel::<UdpMsg>(256);
    tokio::spawn(async move {
        while let Some(reply) = reply_rx.recv().await {
            if to_guest.send(reply).await.is_err() {
                break;
            }
        }
    });

    let mut flows: HashMap<(SocketAddr, SocketAddr), mpsc::Sender<Vec<u8>>> = HashMap::new();
    while let Some((payload, source, destination)) = from_guest.next().await {
        let is_dns = destination.port() == DNS_PORT;
        // Each query is checked, as a flow can carry several.
        if is_dns && let Some(refusal) = policy.check_query(&payload) {
            let _ = reply_tx.try_send((refusal, destination, source));
            continue;
        }
        let payload = match flows.get(&(source, destination)) {
            Some(flow) => match flow.try_send(payload) {
                // A full queue drops the datagram, like a congested link would.
                Ok(()) | Err(TrySendError::Full(_)) => continue,
                // The flow went idle and ended; start a new one.
                Err(TrySendError::Closed(payload)) => payload,
            },
            None => payload,
        };
        let (upstream, idle) = if is_dns {
            (dns, DNS_IDLE)
        } else if policy.allows(destination.ip()) {
            (destination, UDP_IDLE)
        } else {
            eprintln!("Blocked UDP to {destination}");
            continue;
        };
        flows.retain(|_, flow| !flow.is_closed());
        let (flow_tx, flow_rx) = mpsc::channel(64);
        let _ = flow_tx.try_send(payload);
        flows.insert((source, destination), flow_tx);
        tokio::spawn(udp_flow(
            flow_rx,
            reply_tx.clone(),
            (source, destination),
            upstream,
            idle,
            is_dns.then(|| policy.clone()),
        ));
    }
}

/// One guest UDP flow: forwards its datagrams to `upstream` and the answers back to the
/// guest, until it has been idle for `idle`. A DNS flow hands each answer to the policy
/// (`dns`) before the guest gets it.
async fn udp_flow(
    mut from_guest: mpsc::Receiver<Vec<u8>>,
    to_guest: mpsc::Sender<UdpMsg>,
    (guest, destination): (SocketAddr, SocketAddr),
    upstream: SocketAddr,
    idle: Duration,
    dns: Option<Arc<Policy>>,
) {
    let socket = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await {
        Ok(socket) => socket,
        Err(error) => return eprintln!("Couldn't open a UDP socket: {error}"),
    };
    if let Err(error) = socket.connect(upstream).await {
        return eprintln!("Couldn't reach {upstream} over UDP: {error}");
    }
    let mut buffer = vec![0; 65536];
    loop {
        tokio::select! {
            payload = from_guest.recv() => match payload {
                Some(payload) => {
                    let _ = socket.send(&payload).await;
                }
                None => return,
            },
            received = socket.recv(&mut buffer) => match received {
                // Answers appear to come from the address the guest sent to.
                Ok(len) => {
                    if let Some(policy) = &dns {
                        policy.record_answer(&buffer[..len]);
                    }
                    let reply = (buffer[..len].to_vec(), destination, guest);
                    if to_guest.send(reply).await.is_err() {
                        return;
                    }
                }
                // For example "connection refused" after an ICMP port unreachable.
                Err(_) => return,
            },
            _ = sleep(idle) => return,
        }
    }
}

/// The Mac's first IPv4 nameserver, falling back to a public one.
fn upstream_dns() -> SocketAddr {
    let nameserver = std::fs::read_to_string("/etc/resolv.conf")
        .ok()
        .and_then(|conf| {
            conf.lines()
                .filter_map(|line| line.trim().strip_prefix("nameserver"))
                .find_map(|ip| ip.trim().parse::<Ipv4Addr>().ok())
        });
    SocketAddr::from((nameserver.unwrap_or(Ipv4Addr::new(1, 1, 1, 1)), DNS_PORT))
}

/// Socket buffers as Virtualization.framework asks for them: the receive buffer four
/// times the send buffer. They also lift macOS's small default datagram size limit.
fn set_buffers(socket: &StdUnixDatagram) -> io::Result<()> {
    for (option, size) in [(libc::SO_SNDBUF, 1 << 20), (libc::SO_RCVBUF, 4 << 20)] {
        let size: libc::c_int = size;
        let result = unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                option,
                (&raw const size).cast(),
                size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
