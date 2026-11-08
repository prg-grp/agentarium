//! The observer's eBPF programs. They watch the agent and every process it starts, and write
//! what those processes do to the ring buffer `EVENTS`: forks, programs run (and refused),
//! exits, credential changes, file opens, deletes and renames, TCP connections with their
//! outcome, and UDP destinations. They only observe; every program lets the operation pass.
//!
//! A process counts once it runs or forks inside the agent's cgroup (`AGENT`, set by
//! userspace); it then stays in `TRACKED` until it ends, and so do the processes it forks,
//! wherever they move. Everything else returns at once.
//!
//! Noise is left out here, before it costs a ring buffer slot: exec attempts of programs
//! that don't exist, opens a process already made with the same access (`SEEN`), paths the
//! userspace-filled `PATH_FILTER` names, and repeated UDP destinations. `COUNTERS` counts
//! what was left out and what was lost because the ring buffer was full.
//!
//! The kernel structs come from `vmlinux.rs`, generated for the pinned guest kernel by
//! scripts/gen-vmlinux.sh. All kernel memory is read with bpf_probe_read_kernel.

#![no_std]
#![no_main]

#[allow(
    clippy::all,
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unnecessary_transmutes,
    unsafe_op_in_unsafe_fn
)]
#[rustfmt::skip]
mod vmlinux;

use core::ffi::c_void;
use core::mem::offset_of;
use core::ptr::{addr_of, null, null_mut};

use aya_ebpf::bindings;
use aya_ebpf::helpers::generated::{
    bpf_d_path, bpf_get_current_task, bpf_ktime_get_boot_ns, bpf_loop,
    bpf_probe_read_kernel as read_raw, bpf_probe_read_user,
};
use aya_ebpf::helpers::{
    bpf_get_current_pid_tgid, bpf_probe_read_kernel, bpf_probe_read_kernel_str_bytes,
    bpf_probe_read_user_str_bytes,
};
use aya_ebpf::macros::{btf_tracepoint, cgroup_sock_addr, fentry, map, tracepoint};
use aya_ebpf::maps::lpm_trie::Key;
use aya_ebpf::maps::{CgroupArray, HashMap, LpmTrie, LruHashMap, PerCpuArray, RingBuf};
use aya_ebpf::programs::{BtfTracePointContext, FEntryContext, SockAddrContext, TracePointContext};
use guest_observer_common::{
    ARGV_MAX, ConnectEvent, CredEvent, DatagramKey, ExecEvent, ExecFailedEvent, ExitEvent,
    FileEvent, ForkEvent, Header, PATH_MAX, PREFIX_MAX, SeenKey, access, counter, filter, flag,
    kind, protocol, result,
};
use vmlinux::{cred, dentry, file, linux_binprm, mount, path, sock_common, task_struct, vfsmount};

// From the kernel's headers (the same on arm64).
const ENOENT: i64 = 2;
const FMODE_READ: u32 = 0x1;
const FMODE_WRITE: u32 = 0x2;
/// Set on the file a process opens to run it. Kernel 6.8 sets it in `f_flags` (the open
/// flags of exec), not in `f_mode`; both are checked.
const __FMODE_EXEC: u32 = 0x20;
const O_CREAT: u32 = 0o100;
const O_TRUNC: u32 = 0o1000;
const O_APPEND: u32 = 0o2000;
const S_IFMT: u16 = 0o170000;
const S_IFDIR: u16 = 0o040000;
const SIGNAL_GROUP_EXIT: u32 = 0x4;
const AF_INET: u16 = 2;
const AF_INET6: u16 = 10;
const IPPROTO_UDP: u32 = 17;
const TCP_ESTABLISHED: i32 = 1;
const TCP_SYN_SENT: i32 = 2;
const TCP_CLOSE: i32 = 7;
/// The longest name of a path component.
const NAME_MAX: usize = 255;
/// How many path components `write_path` follows.
const PATH_DEPTH: u32 = 48;

// The offsets of the syscall tracepoints' fields (see
// /sys/kernel/tracing/events/syscalls/*/format): after the 8-byte common header and the
// syscall number come the arguments, 8 bytes each.
const EXECVE_FILENAME: usize = 16;
const EXECVEAT_FILENAME: usize = 24;
const EXIT_RET: usize = 16;

#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(16 << 20, 0);

/// Slot 0: the agent's cgroup.
#[map]
static AGENT: CgroupArray = CgroupArray::with_max_entries(1, 0);

/// The processes (thread group IDs) of the agent.
#[map]
static TRACKED: HashMap<u32, u8> = HashMap::with_max_entries(32768, 0);

#[map]
static SEEN: LruHashMap<SeenKey, u8> = LruHashMap::with_max_entries(65536, 0);

#[map]
static DATAGRAMS: LruHashMap<DatagramKey, u8> = LruHashMap::with_max_entries(16384, 0);

/// Path prefixes (their leading bytes) to leave out, with a `filter` value.
#[map]
static PATH_FILTER: LpmTrie<[u8; PREFIX_MAX], u32> = LpmTrie::with_max_entries(64, 0);

#[map]
static COUNTERS: PerCpuArray<u64> = PerCpuArray::with_max_entries(counter::COUNT, 0);

/// An execve in progress: the thread's ID and the user pointer to its filename.
#[map]
static EXECS: LruHashMap<u32, u64> = LruHashMap::with_max_entries(4096, 0);

/// TCP sockets in SYN_SENT: the socket's address and the process that connects.
#[map]
static CONNECTING: LruHashMap<u64, u32> = LruHashMap::with_max_entries(8192, 0);

/// The path walk of `write_path`.
#[map]
static SCRATCH: PerCpuArray<Scratch> = PerCpuArray::with_max_entries(1, 0);

/// Reads a field of kernel memory; `Err` if it can't be read.
macro_rules! read {
    ($place:expr) => {{
        #[allow(unused_unsafe)]
        let value = unsafe { bpf_probe_read_kernel(addr_of!($place)) };
        value
    }};
}

fn count(index: u32) {
    if let Some(counter) = COUNTERS.get_ptr_mut(index) {
        unsafe { *counter += 1 };
    }
}

fn header(kind: u32, tgid: u32) -> Header {
    Header {
        kind,
        tgid,
        time_ns: unsafe { bpf_ktime_get_boot_ns() },
    }
}

fn current_tgid() -> u32 {
    (bpf_get_current_pid_tgid() >> 32) as u32
}

fn tracked(tgid: u32) -> bool {
    unsafe { TRACKED.get(&tgid) }.is_some()
}

/// Whether the current process is the agent's: already tracked, or now in its cgroup.
fn agent_process(tgid: u32) -> bool {
    if tracked(tgid) {
        return true;
    }
    if AGENT.current_task_under_cgroup(0) == Ok(true) {
        let _ = TRACKED.insert(&tgid, &1, 0);
        return true;
    }
    false
}

/// The state of a path walk, shared by `write_path` and each `path_step`. It lives in a map,
/// not on the stack: the verifier doesn't track map memory, so every step starts from the
/// same verifier state and the code after the loop is checked once, not once per step.
#[repr(C)]
struct Scratch {
    /// The `*const dentry` to name next (a map value can't hold pointers).
    dentry: u64,
    /// The `*const mount` it lies on.
    mnt: u64,
    /// Where the path assembled so far starts in `buf`; PATH_MAX while it is empty.
    pos: u64,
    /// 1 once the walk reached the root of everything.
    complete: u64,
    /// The path is assembled from its end; twice PATH_MAX, so every write at an offset
    /// below PATH_MAX stays inside.
    buf: [u8; 2 * PATH_MAX],
}

/// Writes the absolute path of `dentry`, which lies on the mount `vfsmnt`, NUL-terminated to
/// `out` (PATH_MAX bytes). It walks up the dentries and across mount points, writing each
/// name in front of the previous one, like the kernel's d_path. Returns false if the path
/// was cut: too long or too deep; it then keeps its last components.
///
/// The walk runs in `bpf_loop`, so the verifier checks one step instead of PATH_DEPTH
/// unrolled copies, which exceed its limit of a million instructions.
#[inline(always)]
unsafe fn write_path(dentry: *const dentry, vfsmnt: *const vfsmount, out: *mut u8) -> bool {
    let Some(walk) = SCRATCH.get_ptr_mut(0) else {
        unsafe { *out = 0 };
        return false;
    };
    unsafe {
        (*walk).dentry = dentry as u64;
        // A vfsmount is the `mnt` field of its struct mount.
        (*walk).mnt = vfsmnt.cast::<u8>().wrapping_sub(offset_of!(mount, mnt)) as u64;
        (*walk).pos = PATH_MAX as u64;
        (*walk).complete = 0;
        bpf_loop(PATH_DEPTH, path_step as *mut c_void, null_mut(), 0);
    }
    let buf = unsafe { (*walk).buf.as_mut_ptr() };
    let mut pos = (unsafe { (*walk).pos } as usize).min(PATH_MAX);
    if pos == PATH_MAX {
        pos -= 1;
        unsafe { *buf.add(pos & (PATH_MAX - 1)) = b'/' };
    }
    let len = (PATH_MAX - pos) & (PATH_MAX - 1);
    unsafe {
        read_raw(
            out.cast::<c_void>(),
            len as u32,
            buf.add(pos & (PATH_MAX - 1)).cast::<c_void>(),
        );
        *out.add(len) = 0;
        (*walk).complete == 1
    }
}

/// One step of the path walk in SCRATCH: crosses a mount point, or writes the name of the
/// dentry in front of the path. Returns 1 to end the walk.
#[inline(never)]
unsafe extern "C" fn path_step(_index: u64, _ctx: *mut c_void) -> i64 {
    let Some(walk) = SCRATCH.get_ptr_mut(0) else {
        return 1;
    };
    let walk = unsafe { &mut *walk };
    let buf = walk.buf.as_mut_ptr();
    let dentry = walk.dentry as *const dentry;
    let mnt = walk.mnt as *const mount;
    if dentry.is_null() || mnt.is_null() {
        return 1;
    }
    let root = read!((*mnt).mnt.mnt_root).unwrap_or(null_mut_dentry());
    let parent = read!((*dentry).d_parent).unwrap_or(null_mut_dentry());
    if dentry == root.cast_const() || dentry == parent.cast_const() {
        // The root of a filesystem: the path continues where it is mounted, unless this is
        // the root of everything (or a filesystem that isn't mounted).
        let parent_mnt = read!((*mnt).mnt_parent).unwrap_or(core::ptr::null_mut());
        if dentry != root.cast_const() || parent_mnt.cast_const() == mnt || parent_mnt.is_null() {
            walk.complete = 1;
            return 1;
        }
        walk.dentry = read!((*mnt).mnt_mountpoint).unwrap_or(null_mut_dentry()) as u64;
        walk.mnt = parent_mnt as u64;
        return 0;
    }
    let hash_len = read!((*dentry).d_name.__bindgen_anon_1.hash_len).unwrap_or(0);
    let name = read!((*dentry).d_name.name).unwrap_or(null());
    let len = ((hash_len >> 32) as usize) & NAME_MAX;
    let mut pos = walk.pos as usize;
    if pos > PATH_MAX || pos < len + 2 {
        return 1;
    }
    pos -= len;
    unsafe {
        read_raw(
            buf.add(pos & (PATH_MAX - 1)).cast::<c_void>(),
            len as u32,
            name.cast::<c_void>(),
        );
    }
    pos -= 1;
    unsafe { *buf.add(pos & (PATH_MAX - 1)) = b'/' };
    walk.pos = pos as u64;
    walk.dentry = parent as u64;
    0
}

fn null_mut_dentry() -> *mut dentry {
    core::ptr::null_mut()
}

/// Whether the path filter leaves out an access to `path`. Prefixes marked `READS` only
/// leave out accesses that don't write, create, delete or rename.
fn filtered(path: &[u8; PATH_MAX], access: u32) -> bool {
    let mut data = [0u8; PREFIX_MAX];
    data.copy_from_slice(&path[..PREFIX_MAX]);
    let key = Key::new((PREFIX_MAX * 8) as u32, data);
    match PATH_FILTER.get(&key) {
        Some(&filter::ALL) => true,
        Some(&filter::READS) => access & !(access::READ | access::DIRECTORY) == 0,
        _ => false,
    }
}

#[btf_tracepoint(function = "sched_process_fork")]
pub fn sched_process_fork(ctx: BtfTracePointContext) -> i32 {
    let parent: *const task_struct = ctx.arg(0);
    let child: *const task_struct = ctx.arg(1);
    let (Ok(pid), Ok(tgid)) = (read!((*child).pid), read!((*child).tgid)) else {
        return 0;
    };
    // A new thread, not a new process.
    if pid != tgid {
        return 0;
    }
    let parent = read!((*parent).tgid).unwrap_or(0) as u32;
    if !agent_process(parent) {
        return 0;
    }
    let child = tgid as u32;
    let _ = TRACKED.insert(&child, &1, 0);
    let Some(mut entry) = EVENTS.reserve::<ForkEvent>(0) else {
        count(counter::LOST);
        return 0;
    };
    entry.write(ForkEvent {
        header: header(kind::FORK, parent),
        child,
        _pad: 0,
    });
    entry.submit(0);
    0
}

#[btf_tracepoint(function = "sched_process_exec")]
pub fn sched_process_exec(ctx: BtfTracePointContext) -> i32 {
    let task: *const task_struct = ctx.arg(0);
    let bprm: *const linux_binprm = ctx.arg(2);
    let tgid = read!((*task).tgid).unwrap_or(0) as u32;
    if !agent_process(tgid) {
        return 0;
    }
    let Some(mut entry) = EVENTS.reserve::<ExecEvent>(0) else {
        count(counter::LOST);
        return 0;
    };
    let e = entry.as_mut_ptr();
    let parent = read!((*task).real_parent).unwrap_or(core::ptr::null_mut());
    let cred = read!((*task).cred).unwrap_or(null());
    let mut flags = 0;
    unsafe {
        (*e).header = header(kind::EXEC, tgid);
        (*e).ppid = read!((*parent).tgid).unwrap_or(0) as u32;
        (*e).uid = read!((*cred).uid.val).unwrap_or(0);
        (*e).euid = read!((*cred).euid.val).unwrap_or(0);
        (*e).gid = read!((*cred).gid.val).unwrap_or(0);
        (*e).egid = read!((*cred).egid.val).unwrap_or(0);
        (*e)._pad = 0;

        let filename = read!((*bprm).filename).unwrap_or(null());
        if bpf_probe_read_kernel_str_bytes(filename.cast::<u8>(), &mut (*e).filename).is_err() {
            (*e).filename[0] = 0;
        }

        let binary: *const file = read!((*bprm).file).unwrap_or(core::ptr::null_mut());
        let complete = write_path(
            read!((*binary).f_path.dentry).unwrap_or(null_mut_dentry()),
            read!((*binary).f_path.mnt).unwrap_or(core::ptr::null_mut()),
            (*e).binary.as_mut_ptr(),
        );
        let fs = read!((*task).fs).unwrap_or(core::ptr::null_mut());
        let complete = write_path(
            read!((*fs).pwd.dentry).unwrap_or(null_mut_dentry()),
            read!((*fs).pwd.mnt).unwrap_or(core::ptr::null_mut()),
            (*e).cwd.as_mut_ptr(),
        ) && complete;
        if !complete {
            flags |= flag::PATH_TRUNCATED;
        }

        // The new program's arguments, on its stack: NUL-terminated strings, one after the
        // other, between arg_start and arg_end.
        let mm = read!((*task).mm).unwrap_or(core::ptr::null_mut());
        let start = read!((*mm).__bindgen_anon_1.arg_start).unwrap_or(0);
        let end = read!((*mm).__bindgen_anon_1.arg_end).unwrap_or(0);
        let mut len = end.saturating_sub(start) as usize;
        if len >= ARGV_MAX {
            len = ARGV_MAX - 1;
            flags |= flag::ARGV_TRUNCATED;
        }
        let len = len & (ARGV_MAX - 1);
        if bpf_probe_read_user(
            (*e).argv.as_mut_ptr().cast::<c_void>(),
            len as u32,
            start as *const c_void,
        ) < 0
        {
            (*e).argv_len = 0;
        } else {
            (*e).argv_len = len as u32;
        }
        (*e).flags = flags;
    }
    entry.submit(0);
    0
}

/// Remembers the filename of an exec attempt, so its failure can name it.
fn remember_exec(ctx: &TracePointContext, offset: usize) {
    let id = bpf_get_current_pid_tgid();
    if !tracked((id >> 32) as u32) {
        return;
    }
    if let Ok(filename) = unsafe { ctx.read_at::<u64>(offset) } {
        let _ = EXECS.insert(&(id as u32), &filename, 0);
    }
}

/// Reports an exec attempt the kernel refused, unless the program doesn't exist.
fn exec_failed(ctx: &TracePointContext) {
    let id = bpf_get_current_pid_tgid();
    let thread = id as u32;
    let Some(&filename) = (unsafe { EXECS.get(&thread) }) else {
        return;
    };
    let _ = EXECS.remove(&thread);
    let ret = unsafe { ctx.read_at::<i64>(EXIT_RET) }.unwrap_or(0);
    if ret >= 0 {
        return;
    }
    if ret == -ENOENT {
        count(counter::EXEC_NOT_FOUND);
        return;
    }
    let Some(mut entry) = EVENTS.reserve::<ExecFailedEvent>(0) else {
        count(counter::LOST);
        return;
    };
    let e = entry.as_mut_ptr();
    unsafe {
        (*e).header = header(kind::EXEC_FAILED, (id >> 32) as u32);
        (*e).errno = -ret as i32;
        (*e)._pad = 0;
        if bpf_probe_read_user_str_bytes(filename as *const u8, &mut (*e).filename).is_err() {
            (*e).filename[0] = 0;
        }
    }
    entry.submit(0);
}

#[tracepoint(category = "syscalls", name = "sys_enter_execve")]
pub fn sys_enter_execve(ctx: TracePointContext) -> u32 {
    remember_exec(&ctx, EXECVE_FILENAME);
    0
}

#[tracepoint(category = "syscalls", name = "sys_enter_execveat")]
pub fn sys_enter_execveat(ctx: TracePointContext) -> u32 {
    remember_exec(&ctx, EXECVEAT_FILENAME);
    0
}

#[tracepoint(category = "syscalls", name = "sys_exit_execve")]
pub fn sys_exit_execve(ctx: TracePointContext) -> u32 {
    exec_failed(&ctx);
    0
}

#[tracepoint(category = "syscalls", name = "sys_exit_execveat")]
pub fn sys_exit_execveat(ctx: TracePointContext) -> u32 {
    exec_failed(&ctx);
    0
}

#[btf_tracepoint(function = "sched_process_exit")]
pub fn sched_process_exit(ctx: BtfTracePointContext) -> i32 {
    let task: *const task_struct = ctx.arg(0);
    let tgid = read!((*task).tgid).unwrap_or(0) as u32;
    if !tracked(tgid) {
        return 0;
    }
    // Only the last thread of the process reports, once the whole group is dead.
    let signal = read!((*task).signal).unwrap_or(core::ptr::null_mut());
    if read!((*signal).live.counter).unwrap_or(1) != 0 {
        return 0;
    }
    let mut status = read!((*task).exit_code).unwrap_or(0);
    if read!((*signal).flags).unwrap_or(0) & SIGNAL_GROUP_EXIT != 0 {
        status = read!((*signal).group_exit_code).unwrap_or(status);
    }
    let _ = TRACKED.remove(&tgid);
    let Some(mut entry) = EVENTS.reserve::<ExitEvent>(0) else {
        count(counter::LOST);
        return 0;
    };
    entry.write(ExitEvent {
        header: header(kind::EXIT, tgid),
        status,
        _pad: 0,
    });
    entry.submit(0);
    0
}

/// Every change of a process's credentials goes through commit_creds: setuid programs such
/// as sudo at exec, setuid() and the like, capset.
#[fentry(function = "commit_creds")]
pub fn commit_creds(ctx: FEntryContext) -> u32 {
    let tgid = current_tgid();
    if !tracked(tgid) {
        return 0;
    }
    let new: *const cred = ctx.arg(0);
    let task = unsafe { bpf_get_current_task() } as *const task_struct;
    let old = read!((*task).cred).unwrap_or(null());
    let ids = |cred: *const cred| {
        [
            read!((*cred).uid.val).unwrap_or(0),
            read!((*cred).euid.val).unwrap_or(0),
            read!((*cred).gid.val).unwrap_or(0),
            read!((*cred).egid.val).unwrap_or(0),
        ]
    };
    let (before, after) = (ids(old), ids(new));
    let caps_before = read!((*old).cap_effective.val).unwrap_or(0);
    let caps_after = read!((*new).cap_effective.val).unwrap_or(0);
    let raised = caps_after & !caps_before != 0;
    if before == after && !raised {
        return 0;
    }
    let Some(mut entry) = EVENTS.reserve::<CredEvent>(0) else {
        count(counter::LOST);
        return 0;
    };
    entry.write(CredEvent {
        header: header(kind::CRED, tgid),
        uid: [before[0], after[0]],
        euid: [before[1], after[1]],
        gid: [before[2], after[2]],
        egid: [before[3], after[3]],
        caps_raised: raised as u32,
        _pad: 0,
    });
    entry.submit(0);
    0
}

#[fentry(function = "security_file_open")]
pub fn security_file_open(ctx: FEntryContext) -> u32 {
    let tgid = current_tgid();
    if !tracked(tgid) {
        return 0;
    }
    let f: *const file = ctx.arg(0);
    let mode = read!((*f).f_mode).unwrap_or(0);
    let flags = read!((*f).f_flags).unwrap_or(0);
    // Opening the program to run: the exec event reports it.
    if (mode | flags) & __FMODE_EXEC != 0 {
        return 0;
    }
    let inode = read!((*f).f_inode).unwrap_or(core::ptr::null_mut());
    let mut bits = 0;
    if mode & FMODE_READ != 0 {
        bits |= access::READ;
    }
    if mode & FMODE_WRITE != 0 {
        bits |= access::WRITE;
    }
    if flags & O_CREAT != 0 {
        bits |= access::CREATE;
    }
    if flags & O_TRUNC != 0 {
        bits |= access::TRUNCATE;
    }
    if flags & O_APPEND != 0 {
        bits |= access::APPEND;
    }
    if read!((*inode).i_mode).unwrap_or(0) & S_IFMT == S_IFDIR {
        bits |= access::DIRECTORY;
    }
    let sb = read!((*inode).i_sb).unwrap_or(core::ptr::null_mut());
    let seen = SeenKey {
        tgid,
        access: bits,
        dev: read!((*sb).s_dev).unwrap_or(0) as u64,
        ino: read!((*inode).i_ino).unwrap_or(0),
    };
    if unsafe { SEEN.get(&seen) }.is_some() {
        count(counter::REPEATED_OPEN);
        return 0;
    }
    let _ = SEEN.insert(&seen, &1, 0);
    let Some(mut entry) = EVENTS.reserve::<FileEvent>(0) else {
        count(counter::LOST);
        return 0;
    };
    let e = entry.as_mut_ptr();
    unsafe {
        let written = bpf_d_path(
            addr_of!((*f).f_path).cast::<bindings::path>(),
            (*e).path.as_mut_ptr().cast(),
            PATH_MAX as u32,
        );
        if written < 0 {
            (*e).path[0] = 0;
        }
        if filtered(&(*e).path, bits) {
            entry.discard(0);
            count(counter::SYSTEM_PATH);
            return 0;
        }
        (*e).header = header(kind::FILE, tgid);
        (*e).access = bits;
        (*e).flags = if written < 0 { flag::PATH_TRUNCATED } else { 0 };
        (*e).new_path[0] = 0;
    }
    entry.submit(0);
    0
}

/// Reports a delete or a rename; `new` is the target of a rename.
fn path_change(
    tgid: u32,
    dir: *const path,
    dentry: *const dentry,
    new: Option<(*const path, *const dentry)>,
) {
    let Some(mut entry) = EVENTS.reserve::<FileEvent>(0) else {
        count(counter::LOST);
        return;
    };
    let e = entry.as_mut_ptr();
    unsafe {
        let mut complete = write_path(
            dentry,
            read!((*dir).mnt).unwrap_or(core::ptr::null_mut()),
            (*e).path.as_mut_ptr(),
        );
        let bits = match new {
            Some((new_dir, new_dentry)) => {
                complete &= write_path(
                    new_dentry,
                    read!((*new_dir).mnt).unwrap_or(core::ptr::null_mut()),
                    (*e).new_path.as_mut_ptr(),
                );
                access::RENAME
            }
            None => {
                (*e).new_path[0] = 0;
                access::DELETE
            }
        };
        if filtered(&(*e).path, bits) {
            entry.discard(0);
            count(counter::SYSTEM_PATH);
            return;
        }
        (*e).header = header(kind::FILE, tgid);
        (*e).access = bits;
        (*e).flags = if complete { 0 } else { flag::PATH_TRUNCATED };
    }
    entry.submit(0);
}

#[fentry(function = "security_path_unlink")]
pub fn security_path_unlink(ctx: FEntryContext) -> u32 {
    let tgid = current_tgid();
    if tracked(tgid) {
        path_change(tgid, ctx.arg(0), ctx.arg(1), None);
    }
    0
}

#[fentry(function = "security_path_rename")]
pub fn security_path_rename(ctx: FEntryContext) -> u32 {
    let tgid = current_tgid();
    if tracked(tgid) {
        path_change(tgid, ctx.arg(0), ctx.arg(1), Some((ctx.arg(2), ctx.arg(3))));
    }
    0
}

/// TCP: a connect moves the socket to SYN_SENT in the connecting process; the handshake's
/// outcome (ESTABLISHED, or CLOSE) comes later, in any context.
#[btf_tracepoint(function = "inet_sock_set_state")]
pub fn inet_sock_set_state(ctx: BtfTracePointContext) -> i32 {
    let sk: *const sock_common = ctx.arg(0);
    let old: i32 = ctx.arg(1);
    let new: i32 = ctx.arg(2);
    if new == TCP_SYN_SENT {
        let tgid = current_tgid();
        if tracked(tgid) {
            let _ = CONNECTING.insert(&(sk as u64), &tgid, 0);
        }
        return 0;
    }
    if old != TCP_SYN_SENT || (new != TCP_ESTABLISHED && new != TCP_CLOSE) {
        return 0;
    }
    let Some(&tgid) = (unsafe { CONNECTING.get(&(sk as u64)) }) else {
        return 0;
    };
    let _ = CONNECTING.remove(&(sk as u64));
    let family = read!((*sk).skc_family).unwrap_or(0);
    let mut address = [0u8; 16];
    if family == AF_INET6 {
        address = read!((*sk).skc_v6_daddr.in6_u.u6_addr8).unwrap_or(address);
    } else {
        let v4 = read!((*sk).__bindgen_anon_1.__bindgen_anon_1.skc_daddr).unwrap_or(0);
        address[..4].copy_from_slice(&v4.to_ne_bytes());
    }
    let port = read!((*sk).__bindgen_anon_3.__bindgen_anon_1.skc_dport).unwrap_or(0);
    let Some(mut entry) = EVENTS.reserve::<ConnectEvent>(0) else {
        count(counter::LOST);
        return 0;
    };
    entry.write(ConnectEvent {
        header: header(kind::CONNECT, tgid),
        protocol: protocol::TCP,
        result: if new == TCP_ESTABLISHED {
            result::CONNECTED
        } else {
            result::FAILED
        },
        family,
        port: u16::from_be(port),
        _pad: 0,
        address,
    });
    entry.submit(0);
    0
}

/// UDP: the destination of a connect, or of a datagram sent without one. The first per
/// process and destination is reported. TCP is left to `inet_sock_set_state`.
fn datagram(ctx: &SockAddrContext, family: u16) {
    let sa = ctx.sock_addr;
    if unsafe { (*sa).protocol } != IPPROTO_UDP {
        return;
    }
    let tgid = current_tgid();
    if !tracked(tgid) {
        return;
    }
    let mut key = DatagramKey {
        tgid,
        family,
        port: u16::from_be(unsafe { (*sa).user_port } as u16),
        address: [0; 16],
    };
    if key.port == 0 {
        count(counter::NO_PORT);
        return;
    }
    unsafe {
        if family == AF_INET {
            key.address[..4].copy_from_slice(&(*sa).user_ip4.to_ne_bytes());
        } else {
            key.address[..4].copy_from_slice(&(*sa).user_ip6[0].to_ne_bytes());
            key.address[4..8].copy_from_slice(&(*sa).user_ip6[1].to_ne_bytes());
            key.address[8..12].copy_from_slice(&(*sa).user_ip6[2].to_ne_bytes());
            key.address[12..].copy_from_slice(&(*sa).user_ip6[3].to_ne_bytes());
        }
    }
    if unsafe { DATAGRAMS.get(&key) }.is_some() {
        count(counter::REPEATED_DATAGRAM);
        return;
    }
    let _ = DATAGRAMS.insert(&key, &1, 0);
    let Some(mut entry) = EVENTS.reserve::<ConnectEvent>(0) else {
        count(counter::LOST);
        return;
    };
    entry.write(ConnectEvent {
        header: header(kind::CONNECT, tgid),
        protocol: protocol::UDP,
        result: result::SENT,
        family,
        port: key.port,
        _pad: 0,
        address: key.address,
    });
    entry.submit(0);
}

#[cgroup_sock_addr(connect4)]
pub fn connect4(ctx: SockAddrContext) -> i32 {
    datagram(&ctx, AF_INET);
    1
}

#[cgroup_sock_addr(connect6)]
pub fn connect6(ctx: SockAddrContext) -> i32 {
    datagram(&ctx, AF_INET6);
    1
}

#[cgroup_sock_addr(sendmsg4)]
pub fn sendmsg4(ctx: SockAddrContext) -> i32 {
    datagram(&ctx, AF_INET);
    1
}

#[cgroup_sock_addr(sendmsg6)]
pub fn sendmsg6(ctx: SockAddrContext) -> i32 {
    datagram(&ctx, AF_INET6);
    1
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// bpf_probe_read_kernel and bpf_d_path are only available to GPL-compatible programs.
#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
