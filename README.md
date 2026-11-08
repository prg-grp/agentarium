# Agent Toolbox

A local-only, macOS-first Rust desktop app that runs AI agents (Claude Code first) in
persistent Virtualization.framework microVMs. Each agent instance has its own VM settings,
environment variables, host-directory mounts, domain allow/deny policies, an interactive
terminal and activity history. A one-time maintenance boot gives a privileged shell without
the agent.

See [ARCHITECTURE.md](ARCHITECTURE.md) for architecture decisions and [DESIGN.md](DESIGN.md)
for UI/UX guidelines.

## Repository

- `app/` – the desktop app (Rust + [Slint](https://slint.dev) UI)
- `command_plane/` – the protobuf protocol between the app and the guest supervisor, over virtio-vsock
- `data_plane/` – the protobuf protocol from the guest observer to the app, over virtio-vsock
- `guest_supervisor/` – the guest supervisor (`agentarium-supervisor`), a static Linux binary baked into the base rootfs that starts the boot mode the app sends
- `guest_observer/` – the guest observer (`agentarium-observer`), which loads the eBPF programs in agent mode and streams what the agent does to the app
- `guest_observer_ebpf/` – the observer's eBPF programs (aya); `guest_observer_common/` – the event structs both share
- `design/` – per-screen design specs (see DESIGN.md)
- `vms/rootfs/` – Dockerfile for the base guest root filesystem, cloned for every instance
- `vms/provision/` – the presets' provisioning scripts that install an agent harness in an instance, and the custom agent's template
- `scripts/` – helper scripts (guest kernel download, kernel bindings for the eBPF programs, codesign runner)

## Development

Requirements: Apple Silicon Mac, Rust toolchain, Docker (for building the root filesystems).
The guest supervisor and observer are cross-compiled for Linux, which needs the target once:

```bash
rustup target add aarch64-unknown-linux-musl
```

The observer's eBPF programs are built for the BPF target with nightly Rust and
[bpf-linker](https://github.com/aya-rs/bpf-linker):

```bash
rustup toolchain install nightly --component rust-src
```

```bash
cargo binstall bpf-linker
```

The programs read kernel structs through `guest_observer_ebpf/src/vmlinux.rs`, which isn't
committed: `make guest`, `make run` and `make check` generate it from the pinned guest kernel
with `scripts/gen-vmlinux.sh` (needs Docker), and again when the kernel changes.

```bash
make run     # download the guest kernel, build the base rootfs, then cargo run
make guest   # only fetch the kernel and build the base root filesystem into target/
make check   # fmt, cargo check, clippy and script syntax checks
make clean-app  # delete the app's data: database and all instance disks
```

`cargo run` signs the binary ad hoc with the virtualization entitlement before starting it
(see `.cargo/config.toml` and `scripts/codesign-run.sh`). `make run` also sets
`AGENT_TOOLBOX_BASE_ROOTFS`, the base image that instance disks are cloned from. The base
rootfs is rebuilt only when files in `vms/rootfs/base`, the supervisor or the observer change;
`make clean-guest` removes all guest artifacts. Instance disks are clones of the base taken
at creation, so after a base change that existing instances can't use (such as a new
supervisor protocol), run `make clean-app`.
