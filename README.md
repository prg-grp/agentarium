# Agent Toolbox

A local-only, macOS-first Rust desktop app that runs AI agents (Claude Code first) in
persistent Virtualization.framework microVMs. Each agent instance has its own VM settings,
environment variables, host-directory mounts, domain allow/deny policies, an interactive
terminal and activity history. A one-time maintenance boot gives a privileged shell without
the agent.

See [ARCHITECTURE.md](ARCHITECTURE.md) for architecture decisions and [DESIGN.md](DESIGN.md)
for UI/UX guidelines.

## Repository

- `agent-toolbox/` – the desktop app (Rust + [Slint](https://slint.dev) UI)
- `design/` – per-screen design specs (see DESIGN.md)
- `vms/rootfs/` – Dockerfile for the base guest root filesystem, cloned for every instance
- `vms/provision/` – default provisioning scripts that install an agent harness in an instance
- `scripts/` – helper scripts (guest kernel download, codesign runner)

## Development

Requirements: Apple Silicon Mac, Rust toolchain, Docker (for building the root filesystems).

```bash
make run     # download the guest kernel, build the base rootfs, then cargo run
make guest   # only fetch the kernel and build the base root filesystem into target/
make check   # fmt, cargo check, clippy and script syntax checks
make clean-app  # delete the app's data: database and all instance disks
```

`cargo run` signs the binary ad hoc with the virtualization entitlement before starting it
(see `.cargo/config.toml` and `scripts/codesign-run.sh`). `make run` also sets
`AGENT_TOOLBOX_BASE_ROOTFS`, the base image that instance disks are cloned from. The base
rootfs is rebuilt only when files in `vms/rootfs/base` change; `make clean-guest` removes all
guest artifacts.
