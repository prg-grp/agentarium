//! The data plane in the app: what the guest observer saw in an agent boot.
//!
//! The controller connects to the observer's vsock port once the agent session is ready
//! and hands the connection to `spawn`. A thread of its own then reads the observer's
//! messages and stores the events in the database, through a connection of its own, one
//! transaction per batch. It adds what the guest can't know: the subcommand of a program
//! run, and the Mac's path of a file in a shared folder. The UI thread never sees the
//! events; the controller gets a `Summary` with each Health report, every two seconds.

use std::error::Error;
use std::fs::File;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use data_plane::{
    Access, Connection, ConnectionResult, Event, ObserverMessage, Protocol, event, observer_message,
};
use rusqlite::{Transaction, params};

use crate::db;

/// How often a collector_health row is stored while nothing is lost.
const HEALTH_ROW_INTERVAL: Duration = Duration::from_secs(60);

/// The observer's state, sent to the controller with each Health report.
#[derive(Clone, Copy)]
pub struct Summary {
    /// Events lost since the observer started: the kernel's buffer or its queue was full.
    pub lost: u64,
}

/// What the data plane thread tells the controller.
pub enum Observed {
    Health(Summary),
    /// The connection ended; the controller connects again.
    Disconnected,
}

/// The connection to the observer. Dropping it shuts the socket down, which ends its thread.
pub struct Stream(File);

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { libc::shutdown(self.0.as_raw_fd(), libc::SHUT_RDWR) };
    }
}

/// Reads the observer's messages on `socket` on a thread of its own and stores them for
/// the boot `boot`. `mounts` are the shared folders as (guest path, Mac path).
pub fn spawn(
    socket: File,
    boot: i64,
    mounts: Vec<(String, PathBuf)>,
    report: impl Fn(Observed) + Send + 'static,
) -> io::Result<Stream> {
    let reader = socket.try_clone()?;
    std::thread::Builder::new()
        .name("vm-data-plane".into())
        .spawn(move || {
            if let Err(error) = serve(reader, boot, mounts, &report) {
                let closed = error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|error| error.kind() == io::ErrorKind::UnexpectedEof);
                if !closed {
                    eprintln!("The observer's data plane stopped: {error}");
                }
            }
            report(Observed::Disconnected);
        })?;
    Ok(Stream(socket))
}

fn serve(
    mut socket: File,
    boot: i64,
    mut mounts: Vec<(String, PathBuf)>,
    report: &impl Fn(Observed),
) -> Result<(), Box<dyn Error>> {
    let hello = match data_plane::read_message::<ObserverMessage>(&mut socket)?.kind {
        Some(observer_message::Kind::Hello(hello)) => hello,
        _ => return Err("the observer didn't say Hello".into()),
    };
    if hello.protocol_version != data_plane::PROTOCOL_VERSION {
        return Err("the instance's observer doesn't match this app".into());
    }
    // A folder nested in another one wins.
    mounts.sort_by_key(|(guest, _)| std::cmp::Reverse(guest.len()));
    let mut conn = db::open_writer()?;
    // When the last collector_health row was stored, and its loss counters.
    let mut last_stored: Option<(Instant, u64, u64)> = None;
    loop {
        match data_plane::read_message::<ObserverMessage>(&mut socket)?.kind {
            Some(observer_message::Kind::Events(events)) => {
                let tx = conn.transaction()?;
                for event in &events.events {
                    store(&tx, boot, hello.run, &mounts, event)?;
                }
                tx.commit()?;
            }
            Some(observer_message::Kind::Health(health)) => {
                // A row when something was lost, otherwise once a minute: the filter
                // counters change all the time and are only noise.
                let store = last_stored.is_none_or(|(at, lost_kernel, lost_queue)| {
                    (lost_kernel, lost_queue) != (health.lost_kernel, health.lost_queue)
                        || at.elapsed() >= HEALTH_ROW_INTERVAL
                });
                if store {
                    conn.execute(
                        "INSERT INTO collector_health
                         (boot_id, run, lost_kernel, lost_queue, filtered, processes, sequence)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            boot,
                            hello.run as i64,
                            health.lost_kernel as i64,
                            health.lost_queue as i64,
                            serde_json::to_string(&health.filtered)?,
                            health.processes as i64,
                            health.sequence as i64,
                        ],
                    )?;
                    last_stored = Some((Instant::now(), health.lost_kernel, health.lost_queue));
                }
                report(Observed::Health(Summary {
                    lost: health.lost_kernel + health.lost_queue,
                }));
            }
            Some(observer_message::Kind::Hello(_)) | None => {}
        }
    }
}

/// Stores one event.
fn store(
    tx: &Transaction,
    boot: i64,
    run: u64,
    mounts: &[(String, PathBuf)],
    event: &Event,
) -> rusqlite::Result<()> {
    let run = run as i64;
    let execution = event.execution as i64;
    let at = (event.time_unix_ns / 1_000_000) as i64;
    let pid = i64::from(event.pid);
    match &event.kind {
        Some(event::Kind::Exec(exec)) => {
            tx.prepare_cached(
                "INSERT INTO executions (boot_id, run, execution, parent, pid, ppid, started_at,
                 uid, euid, gid, egid, path, binary, argv, argv_truncated, cwd, subcommand,
                 path_truncated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                 ?17, ?18)",
            )?
            .execute(params![
                boot,
                run,
                execution,
                exec.parent_execution as i64,
                pid,
                exec.ppid,
                at,
                exec.uid,
                exec.euid,
                exec.gid,
                exec.egid,
                exec.path,
                exec.binary,
                serde_json::to_string(&exec.argv).unwrap_or_else(|_| "[]".into()),
                exec.argv_truncated,
                exec.cwd,
                subcommand(&exec.path, &exec.argv),
                exec.path_truncated,
            ])?;
        }
        Some(event::Kind::ExecFailed(failed)) => {
            tx.prepare_cached(
                "INSERT INTO executions (boot_id, run, parent, pid, started_at, path, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                boot,
                run,
                failed.parent_execution as i64,
                pid,
                at,
                failed.path,
                failed.errno,
            ])?;
        }
        Some(event::Kind::Exit(exit)) => {
            tx.prepare_cached(
                "UPDATE executions SET exited_at = ?4, exit_code = ?5, exit_signal = ?6
                 WHERE boot_id = ?1 AND run = ?2 AND execution = ?3",
            )?
            .execute(params![boot, run, execution, at, exit.code, exit.signal])?;
        }
        Some(event::Kind::File(file)) => {
            tx.prepare_cached(
                "INSERT INTO file_accesses (boot_id, run, execution, pid, at, path, host_path,
                 access, new_path, path_truncated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?
            .execute(params![
                boot,
                run,
                execution,
                pid,
                at,
                file.path,
                host_path(mounts, &file.path).map(|path| path.to_string_lossy().into_owned()),
                access(file.access),
                (!file.new_path.is_empty()).then_some(&file.new_path),
                file.path_truncated,
            ])?;
        }
        Some(event::Kind::Connection(connection)) => {
            tx.prepare_cached(
                "INSERT INTO connections (boot_id, run, execution, pid, at, protocol, address,
                 port, result)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?
            .execute(params![
                boot,
                run,
                execution,
                pid,
                at,
                match connection.protocol() {
                    Protocol::Tcp => "tcp",
                    Protocol::Udp => "udp",
                    Protocol::Unspecified => "unknown",
                },
                address(connection),
                connection.port,
                match connection.result() {
                    ConnectionResult::Connected => "connected",
                    ConnectionResult::Failed => "failed",
                    ConnectionResult::Sent => "sent",
                    ConnectionResult::Unspecified => "unknown",
                },
            ])?;
        }
        Some(event::Kind::Credentials(cred)) => {
            tx.prepare_cached(
                "INSERT INTO credential_changes (boot_id, run, execution, pid, at, uid_before,
                 uid_after, euid_before, euid_after, gid_before, gid_after, egid_before,
                 egid_after, capabilities_raised)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )?
            .execute(params![
                boot,
                run,
                execution,
                pid,
                at,
                cred.uid_before,
                cred.uid_after,
                cred.euid_before,
                cred.euid_after,
                cred.gid_before,
                cred.gid_after,
                cred.egid_before,
                cred.egid_after,
                cred.capabilities_raised,
            ])?;
        }
        None => {}
    }
    Ok(())
}

/// Programs whose first argument that isn't an option names what they do.
const SUBCOMMANDS: &[&str] = &[
    "apt",
    "apt-get",
    "bun",
    "cargo",
    "deno",
    "docker",
    "gh",
    "git",
    "go",
    "kubectl",
    "npm",
    "npx",
    "pip",
    "pip3",
    "pnpm",
    "poetry",
    "systemctl",
    "uv",
    "uvx",
    "yarn",
];

/// The subcommand of a program run: `status` for `git status`, the module for
/// `python3 -m pip`. None for other programs.
fn subcommand(path: &str, argv: &[String]) -> Option<String> {
    let program = Path::new(path).file_name()?.to_str()?;
    let arguments = argv.get(1..)?;
    if program.starts_with("python") {
        let module = arguments.iter().position(|arg| arg == "-m")?;
        return arguments.get(module + 1).cloned();
    }
    if !SUBCOMMANDS.contains(&program) {
        return None;
    }
    // `git -C <dir>` and `git -c <name=value>` take a value.
    let mut arguments = arguments.iter();
    while let Some(arg) = arguments.next() {
        if program == "git" && (arg == "-C" || arg == "-c") {
            arguments.next();
        } else if !arg.starts_with('-') {
            return Some(arg.clone());
        }
    }
    None
}

/// The Mac's path of a guest path inside a shared folder.
fn host_path(mounts: &[(String, PathBuf)], path: &str) -> Option<PathBuf> {
    mounts.iter().find_map(|(guest, host)| {
        let rest = path.strip_prefix(guest.trim_end_matches('/'))?;
        if rest.is_empty() {
            Some(host.clone())
        } else {
            rest.strip_prefix('/').map(|rest| host.join(rest))
        }
    })
}

/// The access bits as words, e.g. "write,create,truncate".
fn access(bits: u32) -> String {
    [
        (Access::Read, "read"),
        (Access::Write, "write"),
        (Access::Create, "create"),
        (Access::Truncate, "truncate"),
        (Access::Append, "append"),
        (Access::Directory, "directory"),
        (Access::Delete, "delete"),
        (Access::Rename, "rename"),
    ]
    .iter()
    .filter(|(bit, _)| bits & *bit as u32 != 0)
    .map(|(_, word)| *word)
    .collect::<Vec<_>>()
    .join(",")
}

fn address(connection: &Connection) -> String {
    let ip = match connection.address.as_slice() {
        &[a, b, c, d] => IpAddr::V4(Ipv4Addr::new(a, b, c, d)),
        bytes => match <[u8; 16]>::try_from(bytes) {
            Ok(bytes) => IpAddr::V6(Ipv6Addr::from(bytes)),
            Err(_) => return String::new(),
        },
    };
    ip.to_string()
}
