//! What the observer's eBPF programs (guest_observer_ebpf) and its userspace
//! (guest_observer) share: the events the programs write to the ring buffer, and the keys
//! and values of the maps userspace fills.
//!
//! Every event starts with a `Header`, whose `kind` says which struct it is. Strings are
//! NUL-terminated within their buffers.

#![no_std]

/// The longest path the programs report; longer ones are cut and flagged.
pub const PATH_MAX: usize = 4096;
/// The most bytes of a program's arguments (NUL-separated) the programs report.
pub const ARGV_MAX: usize = 4096;
/// How many leading bytes of a path the path filter compares.
pub const PREFIX_MAX: usize = 64;

/// `Header::kind`.
pub mod kind {
    pub const FORK: u32 = 1;
    pub const EXEC: u32 = 2;
    pub const EXEC_FAILED: u32 = 3;
    pub const EXIT: u32 = 4;
    pub const FILE: u32 = 5;
    pub const CONNECT: u32 = 6;
    pub const CRED: u32 = 7;
}

/// `ExecEvent::flags`, `FileEvent::flags`.
pub mod flag {
    pub const ARGV_TRUNCATED: u32 = 1;
    pub const PATH_TRUNCATED: u32 = 2;
}

/// `FileEvent::access`; the same bits as the data plane's `Access`.
pub mod access {
    pub const READ: u32 = 1;
    pub const WRITE: u32 = 2;
    pub const CREATE: u32 = 4;
    pub const TRUNCATE: u32 = 8;
    pub const APPEND: u32 = 16;
    pub const DIRECTORY: u32 = 32;
    pub const DELETE: u32 = 64;
    pub const RENAME: u32 = 128;
}

/// `ConnectEvent::result`.
pub mod result {
    pub const CONNECTED: u8 = 1;
    pub const FAILED: u8 = 2;
    pub const SENT: u8 = 3;
}

/// `ConnectEvent::protocol`: the IP protocol numbers.
pub mod protocol {
    pub const TCP: u8 = 6;
    pub const UDP: u8 = 17;
}

/// Values of the path filter (`PATH_FILTER`, an LPM trie keyed by the path's leading bytes).
pub mod filter {
    /// Leave out every access below the prefix.
    pub const ALL: u32 = 1;
    /// Leave out accesses that only read.
    pub const READS: u32 = 2;
}

/// Indexes of the `COUNTERS` per-CPU array.
pub mod counter {
    /// Events the ring buffer had no room for (lost).
    pub const LOST: u32 = 0;
    /// Exec attempts of programs that don't exist (filtered: shells try every PATH entry).
    pub const EXEC_NOT_FOUND: u32 = 1;
    /// Opens of a file the process already opened with the same access (filtered).
    pub const REPEATED_OPEN: u32 = 2;
    /// Accesses the path filter left out.
    pub const SYSTEM_PATH: u32 = 3;
    /// UDP datagrams to a destination the process already sent to (filtered).
    pub const REPEATED_DATAGRAM: u32 = 4;
    /// UDP sockets connected to port 0: address probes of name lookups, nothing is sent
    /// (filtered).
    pub const NO_PORT: u32 = 5;
    pub const COUNT: u32 = 6;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Header {
    pub kind: u32,
    /// The process (thread group) the event is about.
    pub tgid: u32,
    /// `bpf_ktime_get_boot_ns`: nanoseconds since boot, including suspend.
    pub time_ns: u64,
}

/// A process of the agent forked a new process (not a thread). `header.tgid` is the parent.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ForkEvent {
    pub header: Header,
    pub child: u32,
    pub _pad: u32,
}

/// A process ran a program (sched_process_exec).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExecEvent {
    pub header: Header,
    pub ppid: u32,
    pub uid: u32,
    pub euid: u32,
    pub gid: u32,
    pub egid: u32,
    pub flags: u32,
    /// The bytes of `argv` that hold arguments.
    pub argv_len: u32,
    pub _pad: u32,
    /// The program as the process asked for it (`linux_binprm::filename`).
    pub filename: [u8; PATH_MAX],
    /// The file the kernel ran, as an absolute path.
    pub binary: [u8; PATH_MAX],
    pub cwd: [u8; PATH_MAX],
    /// The arguments, each NUL-terminated.
    pub argv: [u8; ARGV_MAX],
}

/// The kernel refused to run a program for a reason other than ENOENT.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExecFailedEvent {
    pub header: Header,
    /// Positive.
    pub errno: i32,
    pub _pad: u32,
    pub filename: [u8; PATH_MAX],
}

/// The last thread of a process ended.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExitEvent {
    pub header: Header,
    /// The wait status: exit code in bits 8-15, terminating signal in bits 0-6.
    pub status: i32,
    pub _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FileEvent {
    pub header: Header,
    /// `access` bits.
    pub access: u32,
    pub flags: u32,
    pub path: [u8; PATH_MAX],
    /// Renames only: the new path.
    pub new_path: [u8; PATH_MAX],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ConnectEvent {
    pub header: Header,
    pub protocol: u8,
    pub result: u8,
    /// AF_INET or AF_INET6.
    pub family: u16,
    /// Host byte order.
    pub port: u16,
    pub _pad: u16,
    /// IPv4 in the first 4 bytes, or IPv6.
    pub address: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CredEvent {
    pub header: Header,
    pub uid: [u32; 2],
    pub euid: [u32; 2],
    pub gid: [u32; 2],
    pub egid: [u32; 2],
    /// 1 if the effective capabilities gained a capability.
    pub caps_raised: u32,
    pub _pad: u32,
}

/// Key of the `SEEN` map: one open of a file by a process with an access.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SeenKey {
    pub tgid: u32,
    pub access: u32,
    pub dev: u64,
    pub ino: u64,
}

/// Key of the `DATAGRAMS` map: one UDP destination of a process.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DatagramKey {
    pub tgid: u32,
    pub family: u16,
    pub port: u16,
    pub address: [u8; 16],
}

#[cfg(feature = "user")]
mod user {
    use super::*;

    unsafe impl aya::Pod for SeenKey {}
    unsafe impl aya::Pod for DatagramKey {}
}
