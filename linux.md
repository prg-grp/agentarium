# Linux host support

Notes for a later Linux host port. Nothing here is implemented. Terminology follows [ARCHITECTURE.md](ARCHITECTURE.md): **Decided**, **Proposed**, **Open decision**.

## Decided

- The app ships on macOS and Linux. Windows is out of scope; it is not merely deferred.
- macOS stays the first platform. The Linux port starts once the macOS release is in place.

Why Windows was dropped: the only fitting hypervisor there is Hyper-V (Host Compute Service), which has no virtio devices. Disks (VHDX), consoles (COM ports over named pipes), the command plane (hv_sock with registry-registered service GUIDs), shared folders (Plan 9 or SMB instead of virtio-fs) and the network dataplane (no raw-frame NIC) would all need a second implementation, plus a privileged helper for VM creation. WSL2 doesn't fit either: all distros share one VM, its host-drive mounts are configured inside the guest, and its network isn't mediated by the app. QEMU with WHPX lacks vsock and virtio-fs on a Windows host.

## What carries over unchanged

With macOS and Linux both Unix and both giving the guest virtio devices, most of the design stays:

- **Guest image and supervisor:** the two virtio consoles (`hvc0` kernel console, `hvc1` session), virtio-vsock on port 700, the virtio-fs mount, systemd, cgroup delegation and the boot modes. The vsock transport module (`vmw_vsock_virtio_transport`) stays the same, whatever the VMM does on the host side. The privileged-port rule (only root listens on ports ≤ 1023) is part of the Linux vsock core, not of a transport.
- **Stop:** the command plane `Stop` works everywhere. The power-button fallback works on QEMU's arm `virt` machine (also a PL061 GPIO key) and via ACPI on x86.
- **Clock:** KVM gives the guest kvm-clock and an RTC. `clock-from-host` is harmless there and can stay.
- **Host code:** `std::os::unix` (pipes, `UnixDatagram`, `fcntl`) exists on Linux, so the socketpair in `app/src/net.rs`, the smoltcp stack, `app/src/policy.rs` and the console handling in `app/src/terminal.rs` carry over.
- **UI:** Slint and winit run on Linux.
- **Database:** `rusqlite` with bundled SQLite and the migrations are platform-independent.

## Mapping of the Mac-specific pieces

| Piece | macOS today | Linux |
|---|---|---|
| Hypervisor | Virtualization.framework in the app process (`app/src/vm.rs`, `objc2-virtualization`) | KVM through a VMM, see [VMM](#vmm) |
| Boot | `VZLinuxBootLoader`, arm64 `Image` | Direct kernel boot; arm64 `Image` or x86_64 `bzImage`/`vmlinux` |
| Root disk | Sparse raw ext4, instance disk via `clonefile` | See [Instance disks](#instance-disks) |
| Consoles | Two virtio consoles over pipes | virtio-console, same guest devices |
| Command plane | virtio-vsock (`VZVirtioSocketDevice`) | Kernel vhost-vsock (`AF_VSOCK` + guest CID, needs `/dev/vhost-vsock`) with QEMU, or vsock bridged to a Unix socket (libkrun, Cloud Hypervisor, Firecracker). Guest unchanged either way |
| Shared folders | One virtio-fs device with `VZMultipleDirectoryShare`; read-only enforced by the Mac | See [Shared folders](#shared-folders) |
| Network | `VZFileHandleNetworkDeviceAttachment`: raw Ethernet frames over a datagram socketpair | QEMU (`-netdev dgram`/`stream` on a Unix socket) and libkrun (unixgram fd) carry the same frames: a near 1:1 replacement |
| Data folder | `~/Library/Application Support/...` via `NSFileManager` (`app/src/db.rs`) | XDG data directory (`$XDG_DATA_HOME`, usually `~/.local/share/...`) |
| Host resources | `sysctlbyname("hw.memsize")` (`app/src/editor.rs`) | `/proc/meminfo` or `sysinfo(2)` |
| App integration | Dock icon and `applicationShouldTerminate:` via objc2 (`app/src/main.rs`) | Not needed; quit handling through winit/Slint |
| Credentials (planned) | Keychain | Secret Service (for example the `keyring` crate) |
| Entitlement | `com.apple.security.virtualization` | Read/write access to `/dev/kvm` (usually the `kvm` group) |

## Open decisions

### VMM

Candidates, measured against what the app needs (virtio-fs, vsock, raw frames into the userspace network stack, direct kernel boot):

| VMM | virtio-fs | vsock | Raw frames to the app | Notes |
|---|---|---|---|---|
| **libkrun** | Built in (no `virtiofsd`) | Bridged to a Unix socket | Yes (unixgram fd) | Rust, used by krunvm, muvm and Podman. Takes over the calling process, so one helper process per VM. Also runs on macOS (Hypervisor.framework) |
| **QEMU** | Via `virtiofsd` (vhost-user) | Kernel vhost-vsock | Yes (`-netdev dgram`/`stream`) | Has everything, heavier, one process per VM |
| Cloud Hypervisor | Via `virtiofsd` | Bridged to a Unix socket | No: TAP (needs `CAP_NET_ADMIN`) or a vhost-user-net backend the app would implement | Rust, REST API |
| Firecracker | **None** | Bridged to a Unix socket | No: TAP only | Rules out live host-directory mounts |

Proposed: libkrun or QEMU. Firecracker is out (no virtio-fs); Cloud Hypervisor only if the app takes on a vhost-user-net backend.

### One backend or two

- **Two backends:** Virtualization.framework stays on macOS; Linux gets its own backend behind the same interface as `app/src/vm.rs` (create, start, request stop, stop, connect, state), selected with `cfg`.
- **One backend:** libkrun on both. One code path, but it replaces a mature Apple stack on the platform that ships first, and adds a helper process per VM on macOS too.

Proposed: two backends. Revisit libkrun for both only if a spike shows it covers read-only multi-folder virtio-fs, vsock and the network fd well on both systems.

### Instance disks

`clonefile` has no general Linux equivalent, and ARCHITECTURE.md decides "no copy fallback".

- **Reflink** (`FICLONE` ioctl, `cp --reflink`): same behaviour as `clonefile`, but only on Btrfs, XFS (with reflink) and bcachefs, not on ext4, the most common Linux filesystem.
- **qcow2 overlay** on the base image: works on any filesystem, but the disk is no longer raw ext4. Supported by QEMU; check libkrun.
- **Sparse copy** (`cp --sparse=always`): works everywhere, but copies the base's written blocks (a few GB) per instance and gives up block sharing.

Open: which of these, and whether "no copy fallback" stays macOS-only. A possible order: reflink, else qcow2 overlay.

### Shared folders

There is no direct counterpart to `VZMultipleDirectoryShare` (several host folders under one virtio-fs tag).

- `virtiofsd` serves one directory per instance of it. Several folders mean one virtio-fs device (tag) per folder, or one parent directory assembled from bind mounts on the host (needs a mount namespace or privileges).
- With one tag per folder, `Start.mounts` names a tag instead of a subfolder of `terrarium-shares`, and the supervisor mounts each tag directly. That changes the command plane and the supervisor for both platforms, or only for Linux if the Mac keeps one tag.
- Read-only must be enforced on the host, outside guest root (today: read-only `VZSharedDirectory`). To verify for the chosen VMM: `virtiofsd` read-only mode, libkrun's read-only support per share.
- `virtiofsd` can run unprivileged with a user-namespace sandbox. Check UID/GID mapping, so files the guest's `ubuntu` user writes end up owned by the host user.

Open: one tag per folder vs. a host-side assembled directory; which mechanism enforces read-only.

### Guest architecture

Most Linux hosts are x86_64; today only arm64 is built.

- An x86_64 kernel (pinned and checksummed like `scripts/build-guest.sh`), an amd64 base rootfs (`docker build --platform linux/amd64`, the matching `linux-modules` package) and the supervisor for `x86_64-unknown-linux-musl`.
- Open: whether arm64 Linux hosts are supported too (the arm64 guest already exists), and how a released app picks and obtains the right guest bundle (already open in ARCHITECTURE.md for macOS).

### Privileges and packaging

- `/dev/kvm` access; `/dev/vhost-vsock` only with QEMU's kernel vsock.
- No TAP and no `CAP_NET_ADMIN` as long as the VMM hands raw frames to the app, which keeps network enforcement in the app as on macOS.
- Open: packaging (Flatpak sandboxing gets in the way of `/dev/kvm` and helper processes; AppImage, `.deb`/`.rpm` or a tarball), and whether the VMM (QEMU, libkrun) is bundled or a system dependency.

### Observability

The planned guest collector (eBPF or audit) runs in the guest and is independent of the host OS. Host-side network observations come from the app's own network stack on both platforms. Nothing Linux-specific is expected here; to confirm once the collector exists.

## Before starting the port

- Keep everything Apple-specific behind the narrow interface it has today (`app/src/vm.rs`, disk creation, data folder, host resources, `main.rs` app integration). Add the Linux side behind `cfg(target_os = "linux")` only when the port starts; don't abstract ahead of time.
- A short spike on the VMM choice: boot the existing guest image under libkrun (and/or QEMU), check vsock port 700, virtio-fs with several read-only and read-write folders, and the network fd with the existing `net.rs` stack.
- Unverified details in this file come from general knowledge, not from tests or current documentation. Check them during the spike: libkrun's read-only virtio-fs and qcow2 support, `virtiofsd` options, and Cloud Hypervisor's networking options.
