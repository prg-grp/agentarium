//! agentarium-observer: watches what the agent does and streams it to the app.
//!
//! The guest supervisor starts it in agent mode only, as root, with the agent's cgroup:
//! `agentarium-observer --cgroup <dir>`. It loads the eBPF programs of guest_observer_ebpf,
//! points them at that cgroup, fills their path filter, attaches them, and prints `ready` on
//! stdout; the supervisor starts the agent after that. A program the kernel refuses is
//! logged and left out; the others keep observing.
//!
//! The main thread reads the programs' ring buffer and turns the raw events into data plane
//! events. It keeps a table of the agent's processes to number every program run (an
//! execution) and to link it to the execution that started it, through forks. The events
//! wait in a bounded queue; a second thread serves the app on the data plane's vsock port,
//! sending them in batches and Health every HEALTH_INTERVAL.
//!
//! Logs go to stderr (the journal and the Logs tab) and name no paths or arguments.
//! Built for the Linux guest only (see the Makefile).

use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use aya::maps::lpm_trie::{Key, LpmTrie};
use aya::maps::{CgroupArray, MapData, PerCpuArray, RingBuf};
use aya::programs::{BtfTracePoint, CgroupAttachMode, CgroupSockAddr, FEntry, Program, TracePoint};
use aya::{Btf, Ebpf};
use data_plane::{
    Connection, ConnectionResult, CredentialChange, Event, Events, Exec, ExecFailed, Exit,
    FileAccess, Health, Hello, Message, ObserverMessage, Protocol, event, observer_message,
};
use guest_observer_common::{
    ConnectEvent, CredEvent, ExecEvent, ExecFailedEvent, ExitEvent, FileEvent, ForkEvent, Header,
    PREFIX_MAX, counter, filter, flag, kind, protocol, result,
};
use tracing::{error, info, warn};

/// Events waiting for the app; when it is full, new events are lost (and counted).
const QUEUE_LIMIT: usize = 100_000;
/// How long a batch collects events before it is sent.
const BATCH_INTERVAL: Duration = Duration::from_millis(200);
/// The most encoded bytes of events per batch; well below the frame limit.
const BATCH_BYTES: usize = 512 * 1024;
/// How long the reader waits for the ring buffer before it looks again.
const POLL_TIMEOUT_MS: i32 = 1000;

/// What the path filter leaves out (see guest_observer_ebpf). The prefixes are compared on
/// their bytes, so directories end with a slash.
const PATH_FILTER: &[(&str, u32)] = &[
    ("/proc/", filter::ALL),
    ("/sys/", filter::ALL),
    ("/dev/", filter::ALL),
    ("/usr/lib/", filter::READS),
    ("/lib/", filter::READS),
    ("/usr/share/", filter::READS),
    ("/etc/ld.so.cache", filter::READS),
    ("/etc/localtime", filter::READS),
];

/// The programs, by the kind of hook they attach to.
enum Hook {
    /// A BTF tracepoint; the program's name is the tracepoint's.
    TpBtf,
    /// A kernel function; the program's name is the function's.
    FEntry,
    /// A classic tracepoint in this category; the program's name is the tracepoint's.
    TracePoint(&'static str),
    /// A cgroup socket-address hook, attached to the root cgroup.
    SockAddr,
}

const PROGRAMS: &[(&str, Hook)] = &[
    ("sched_process_fork", Hook::TpBtf),
    ("sched_process_exec", Hook::TpBtf),
    ("sched_process_exit", Hook::TpBtf),
    ("inet_sock_set_state", Hook::TpBtf),
    ("commit_creds", Hook::FEntry),
    ("security_file_open", Hook::FEntry),
    ("security_path_unlink", Hook::FEntry),
    ("security_path_rename", Hook::FEntry),
    ("sys_enter_execve", Hook::TracePoint("syscalls")),
    ("sys_enter_execveat", Hook::TracePoint("syscalls")),
    ("sys_exit_execve", Hook::TracePoint("syscalls")),
    ("sys_exit_execveat", Hook::TracePoint("syscalls")),
    ("connect4", Hook::SockAddr),
    ("connect6", Hook::SockAddr),
    ("sendmsg4", Hook::SockAddr),
    ("sendmsg6", Hook::SockAddr),
];

/// The names of `counter` indexes that count filtered events, as Health reports them.
const FILTERED: &[(u32, &str)] = &[
    (counter::EXEC_NOT_FOUND, "exec_not_found"),
    (counter::REPEATED_OPEN, "repeated_open"),
    (counter::SYSTEM_PATH, "system_path"),
    (counter::REPEATED_DATAGRAM, "repeated_datagram"),
    (counter::NO_PORT, "no_port"),
];

/// What the reader and the sender share.
#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    /// Signalled when an event is queued.
    queued: Condvar,
    /// Processes of the agent alive now.
    processes: AtomicU64,
    /// Names this run of the observer (Hello.run).
    run: u64,
}

#[derive(Default)]
struct Queue {
    events: VecDeque<Event>,
    /// The last sequence number given out, sent or lost.
    sequence: u64,
    /// Events that didn't fit into the queue.
    lost: u64,
}

impl Shared {
    fn push(&self, mut event: Event) {
        let mut queue = self.queue.lock().unwrap();
        queue.sequence += 1;
        event.sequence = queue.sequence;
        if queue.events.len() >= QUEUE_LIMIT {
            queue.lost += 1;
            return;
        }
        queue.events.push_back(event);
        self.queued.notify_one();
    }

    /// Waits up to BATCH_INTERVAL for events, then takes as many as fit into a batch.
    fn take_batch(&self) -> Vec<Event> {
        let queue = self.queue.lock().unwrap();
        let (mut queue, _) = self
            .queued
            .wait_timeout_while(queue, BATCH_INTERVAL, |queue| queue.events.is_empty())
            .unwrap();
        let mut batch = Vec::new();
        let mut bytes = 0;
        while let Some(event) = queue.events.front() {
            bytes += event.encoded_len() + 8;
            if bytes > BATCH_BYTES && !batch.is_empty() {
                break;
            }
            batch.extend(queue.events.pop_front());
        }
        batch
    }

    /// Puts a batch that couldn't be sent back in front of the queue.
    fn put_back(&self, batch: Vec<Event>) {
        let mut queue = self.queue.lock().unwrap();
        for event in batch.into_iter().rev() {
            queue.events.push_front(event);
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(io::stderr)
        .init();
    if let Err(error) = run() {
        error!(%error, "the observer stopped");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let cgroup = match (args.next().as_deref(), args.next()) {
        (Some("--cgroup"), Some(dir)) => dir,
        _ => return Err("usage: agentarium-observer --cgroup <dir>".into()),
    };
    info!(
        version = env!("CARGO_PKG_VERSION"),
        "agentarium-observer starting"
    );

    // Kernels before 5.11 charge BPF maps to the locked-memory limit.
    let unlimited = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &unlimited) };

    let mut ebpf = Ebpf::load(aya::include_bytes_aligned!(concat!(
        env!("OUT_DIR"),
        "/guest_observer_ebpf"
    )))?;
    let mut agent = CgroupArray::try_from(ebpf.map_mut("AGENT").ok_or("no AGENT map")?)?;
    agent.set(0, File::open(&cgroup)?, 0)?;
    let mut paths: LpmTrie<_, [u8; PREFIX_MAX], u32> =
        LpmTrie::try_from(ebpf.map_mut("PATH_FILTER").ok_or("no PATH_FILTER map")?)?;
    for &(prefix, value) in PATH_FILTER {
        let mut data = [0; PREFIX_MAX];
        data[..prefix.len()].copy_from_slice(prefix.as_bytes());
        paths.insert(&Key::new(prefix.len() as u32 * 8, data), &value, 0)?;
    }

    let btf = Btf::from_sys_fs()?;
    let root_cgroup = File::open("/sys/fs/cgroup")?;
    let mut attached = 0;
    for (name, hook) in PROGRAMS {
        match attach(&mut ebpf, name, hook, &btf, &root_cgroup) {
            Ok(()) => attached += 1,
            Err(error) => {
                // The causes hold the errno. A verifier rejection carries the whole verifier
                // log; its last lines say why.
                let mut message = error.to_string();
                let mut source = error.source();
                while let Some(cause) = source {
                    message = format!("{message}: {cause}");
                    source = cause.source();
                }
                let lines: Vec<&str> = message.lines().collect();
                let error = if lines.len() > 6 {
                    format!("{} … {}", lines[0], lines[lines.len() - 4..].join(" | "))
                } else {
                    lines.join(" | ")
                };
                warn!(
                    program = name,
                    error, "couldn't attach a program; it doesn't observe"
                )
            }
        }
    }
    if attached == 0 {
        return Err("no program could be attached".into());
    }
    info!(attached, of = PROGRAMS.len(), "attached the programs");

    let events = RingBuf::try_from(ebpf.take_map("EVENTS").ok_or("no EVENTS map")?)?;
    let counters: PerCpuArray<MapData, u64> =
        PerCpuArray::try_from(ebpf.take_map("COUNTERS").ok_or("no COUNTERS map")?)?;

    let shared = Arc::new(Shared {
        run: clock_ns(libc::CLOCK_BOOTTIME) as u64,
        ..Shared::default()
    });
    let sender = shared.clone();
    thread::Builder::new()
        .name("data-plane".into())
        .spawn(move || serve(&sender, &counters))?;

    // The supervisor starts the agent once it reads this.
    let mut stdout = io::stdout();
    writeln!(stdout, "ready")?;
    stdout.flush()?;

    read(events, &shared)
}

/// Loads and attaches one program.
fn attach(
    ebpf: &mut Ebpf,
    name: &str,
    hook: &Hook,
    btf: &Btf,
    root_cgroup: &File,
) -> Result<(), Box<dyn Error>> {
    let program: &mut Program = ebpf.program_mut(name).ok_or("the program is missing")?;
    match hook {
        Hook::TpBtf => {
            let program: &mut BtfTracePoint = program.try_into()?;
            program.load(name, btf)?;
            program.attach()?;
        }
        Hook::FEntry => {
            let program: &mut FEntry = program.try_into()?;
            program.load(name, btf)?;
            program.attach()?;
        }
        Hook::TracePoint(category) => {
            let program: &mut TracePoint = program.try_into()?;
            program.load()?;
            program.attach(category, name)?;
        }
        Hook::SockAddr => {
            let program: &mut CgroupSockAddr = program.try_into()?;
            program.load()?;
            // aya attaches through a bpf_link, which shares the cgroup with other programs
            // anyway; the kernel refuses the ALLOW_MULTI flag on a link (EINVAL).
            program.attach(root_cgroup.as_fd(), CgroupAttachMode::Single)?;
        }
    }
    Ok(())
}

/// A process of the agent, as the reader tracks it.
#[derive(Clone, Copy)]
struct Process {
    /// The execution it runs: its own after an exec, its parent's after a fork.
    execution: u64,
    /// Whether it ran a program itself, so its exit ends `execution`.
    exec: bool,
}

/// Reads the ring buffer for as long as the observer runs.
fn read(mut events: RingBuf<MapData>, shared: &Shared) -> Result<(), Box<dyn Error>> {
    let mut processes: HashMap<u32, Process> = HashMap::new();
    let mut executions = 0;
    loop {
        let mut poll = libc::pollfd {
            fd: events.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, POLL_TIMEOUT_MS) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error.into());
            }
        }
        // Kernel times count from boot; the guest's clock may have been set meanwhile.
        let offset = clock_ns(libc::CLOCK_REALTIME) - clock_ns(libc::CLOCK_BOOTTIME);
        while let Some(item) = events.next() {
            if let Some(event) = convert(&item, &mut processes, &mut executions, offset) {
                shared.push(event);
            }
        }
        shared
            .processes
            .store(processes.len() as u64, Ordering::Relaxed);
    }
}

/// Turns a raw event into a data plane event, keeping the process table up to date. Forks
/// only update the table.
fn convert(
    raw: &[u8],
    processes: &mut HashMap<u32, Process>,
    executions: &mut u64,
    offset: i128,
) -> Option<Event> {
    let header: Header = cast(raw)?;
    let tgid = header.tgid;
    let current = processes.get(&tgid).map_or(0, |process| process.execution);
    let mut event = Event {
        sequence: 0,
        time_unix_ns: (i128::from(header.time_ns) + offset).max(0) as u64,
        execution: current,
        pid: tgid,
        kind: None,
    };
    event.kind = Some(match header.kind {
        kind::FORK => {
            let fork: ForkEvent = cast(raw)?;
            processes.insert(
                fork.child,
                Process {
                    execution: current,
                    exec: false,
                },
            );
            return None;
        }
        kind::EXEC => {
            let exec: Box<ExecEvent> = cast_boxed(raw)?;
            *executions += 1;
            event.execution = *executions;
            processes.insert(
                tgid,
                Process {
                    execution: *executions,
                    exec: true,
                },
            );
            let cwd = text(&exec.cwd);
            let argv = exec.argv[..(exec.argv_len as usize).min(exec.argv.len())]
                .split(|&byte| byte == 0)
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect::<Vec<_>>();
            // The last argument ends with a NUL, which leaves an empty piece behind it.
            let argv = match argv.split_last() {
                Some((last, rest)) if last.is_empty() => rest.to_vec(),
                _ => argv,
            };
            event::Kind::Exec(Exec {
                parent_execution: current,
                ppid: exec.ppid,
                uid: exec.uid,
                euid: exec.euid,
                gid: exec.gid,
                egid: exec.egid,
                path: absolute(&text(&exec.filename), &cwd),
                binary: text(&exec.binary),
                argv,
                argv_truncated: exec.flags & flag::ARGV_TRUNCATED != 0,
                cwd,
                path_truncated: exec.flags & flag::PATH_TRUNCATED != 0,
            })
        }
        kind::EXEC_FAILED => {
            let failed: Box<ExecFailedEvent> = cast_boxed(raw)?;
            event::Kind::ExecFailed(ExecFailed {
                parent_execution: current,
                path: text(&failed.filename),
                errno: failed.errno,
            })
        }
        kind::EXIT => {
            let exit: ExitEvent = cast(raw)?;
            let process = processes.remove(&tgid)?;
            // A fork that never ran a program doesn't end its parent's execution.
            if !process.exec {
                return None;
            }
            let signal = exit.status & 0x7f;
            event::Kind::Exit(Exit {
                code: if signal == 0 {
                    (exit.status >> 8) & 0xff
                } else {
                    -1
                },
                signal,
            })
        }
        kind::FILE => {
            let file: Box<FileEvent> = cast_boxed(raw)?;
            event::Kind::File(FileAccess {
                path: text(&file.path),
                access: file.access,
                new_path: text(&file.new_path),
                path_truncated: file.flags & flag::PATH_TRUNCATED != 0,
            })
        }
        kind::CONNECT => {
            let connect: ConnectEvent = cast(raw)?;
            let address = if connect.family == libc::AF_INET6 as u16 {
                connect.address.to_vec()
            } else {
                connect.address[..4].to_vec()
            };
            event::Kind::Connection(Connection {
                protocol: match connect.protocol {
                    protocol::TCP => Protocol::Tcp,
                    protocol::UDP => Protocol::Udp,
                    _ => Protocol::Unspecified,
                }
                .into(),
                address,
                port: connect.port.into(),
                result: match connect.result {
                    result::CONNECTED => ConnectionResult::Connected,
                    result::FAILED => ConnectionResult::Failed,
                    result::SENT => ConnectionResult::Sent,
                    _ => ConnectionResult::Unspecified,
                }
                .into(),
            })
        }
        kind::CRED => {
            let cred: CredEvent = cast(raw)?;
            event::Kind::Credentials(CredentialChange {
                uid_before: cred.uid[0],
                uid_after: cred.uid[1],
                euid_before: cred.euid[0],
                euid_after: cred.euid[1],
                gid_before: cred.gid[0],
                gid_after: cred.gid[1],
                egid_before: cred.egid[0],
                egid_after: cred.egid[1],
                capabilities_raised: cred.caps_raised != 0,
            })
        }
        _ => return None,
    });
    Some(event)
}

/// Copies a raw event into `T`; None if it is too short.
fn cast<T: Copy>(raw: &[u8]) -> Option<T> {
    (raw.len() >= size_of::<T>()).then(|| unsafe { raw.as_ptr().cast::<T>().read_unaligned() })
}

/// Like `cast`, for the large events (kilobytes of paths), which stay off the stack.
fn cast_boxed<T: Copy>(raw: &[u8]) -> Option<Box<T>> {
    if raw.len() < size_of::<T>() {
        return None;
    }
    let mut boxed = Box::<T>::new_uninit();
    unsafe {
        std::ptr::copy_nonoverlapping(
            raw.as_ptr(),
            boxed.as_mut_ptr().cast::<u8>(),
            size_of::<T>(),
        );
        Some(boxed.assume_init())
    }
}

/// The text before the first NUL.
fn text(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// `path` made absolute with `cwd`, if it is relative and `cwd` is known.
fn absolute(path: &str, cwd: &str) -> String {
    if path.starts_with('/') || cwd.is_empty() {
        return path.to_owned();
    }
    let path = path.strip_prefix("./").unwrap_or(path);
    format!("{}/{path}", cwd.trim_end_matches('/'))
}

fn clock_ns(clock: libc::clockid_t) -> i128 {
    let mut now: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(clock, &mut now) };
    i128::from(now.tv_sec) * 1_000_000_000 + i128::from(now.tv_nsec)
}

/// Serves the app on the data plane's port, one connection at a time, for as long as the
/// observer runs.
fn serve(shared: &Shared, counters: &PerCpuArray<MapData, u64>) {
    // The vsock transport module is loaded at boot, before the supervisor starts this.
    let listener = loop {
        match data_plane::vsock::listen(data_plane::PORT) {
            Ok(listener) => break listener,
            Err(error) => {
                warn!(port = data_plane::PORT, %error, "couldn't listen on vsock, trying again");
                thread::sleep(Duration::from_secs(1));
            }
        }
    };
    info!(port = data_plane::PORT, "listening for the app on vsock");
    loop {
        match data_plane::vsock::accept(&listener) {
            Ok((connection, ..)) => {
                info!("the app connected");
                match stream(connection, shared, counters) {
                    Ok(()) => {}
                    Err(error) => info!(%error, "the app disconnected"),
                }
            }
            Err(error) => {
                warn!(%error, "couldn't accept a connection");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

/// Sends Hello, then the queued events and Health, until the connection fails.
fn stream(
    mut connection: File,
    shared: &Shared,
    counters: &PerCpuArray<MapData, u64>,
) -> io::Result<()> {
    let send = |connection: &mut File, kind| {
        data_plane::write_message(connection, &ObserverMessage { kind: Some(kind) })
    };
    send(
        &mut connection,
        observer_message::Kind::Hello(Hello {
            protocol_version: data_plane::PROTOCOL_VERSION,
            run: shared.run,
        }),
    )?;
    let mut next_health = Instant::now();
    loop {
        let batch = shared.take_batch();
        if !batch.is_empty() {
            let events = Events { events: batch };
            if let Err(error) = send(
                &mut connection,
                observer_message::Kind::Events(events.clone()),
            ) {
                shared.put_back(events.events);
                return Err(error);
            }
        }
        if Instant::now() >= next_health {
            next_health = Instant::now() + data_plane::HEALTH_INTERVAL;
            send(
                &mut connection,
                observer_message::Kind::Health(health(shared, counters)),
            )?;
        }
    }
}

fn health(shared: &Shared, counters: &PerCpuArray<MapData, u64>) -> Health {
    // Each counter is per CPU; a counter that can't be read is 0.
    let total = |index: u32| -> u64 {
        counters
            .get(&index, 0)
            .map(|values| values.iter().sum())
            .unwrap_or(0)
    };
    let queue = shared.queue.lock().unwrap();
    Health {
        lost_kernel: total(counter::LOST),
        lost_queue: queue.lost,
        filtered: FILTERED
            .iter()
            .map(|&(index, name)| (name.to_owned(), total(index)))
            .collect(),
        processes: shared.processes.load(Ordering::Relaxed),
        sequence: queue.sequence,
    }
}
