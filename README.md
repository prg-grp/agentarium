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
- `vms/rootfs/` – Dockerfiles for the guest root filesystems (`base`, `claude`)
- `scripts/` – helper scripts (guest kernel download, codesign runner)

## Development

Requirements: Apple Silicon Mac, Rust toolchain, Docker (for building the root filesystems).

```bash
make run     # download the guest kernel, build root filesystems, then cargo run
make guest   # only fetch the kernel and build the root filesystems into target/
make check   # fmt, cargo check, clippy and script syntax checks
```

`cargo run` signs the binary ad hoc with the virtualization entitlement before starting it
(see `.cargo/config.toml` and `scripts/codesign-run.sh`). Root filesystems are rebuilt only
when files in their directory change; `make clean-guest` removes all guest artifacts.
