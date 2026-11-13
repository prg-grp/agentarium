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
- `guest_supervisor/` – the guest supervisor (`agentarium-supervisor`), a static Linux binary the app shares read-only with every boot that starts the boot mode the app sends
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
make run     # download the guest kernel, build the base rootfs and guest binaries, then cargo run
make guest   # only fetch the kernel and build the base rootfs and guest binaries into target/
make guest-bin  # only build the supervisor and observer into target/guest/bin
make check   # fmt, cargo check, clippy and script syntax checks
make clean-app  # delete the app's data: database and all instance disks
```

`cargo run` signs the binary ad hoc with the virtualization entitlement before starting it
(see `.cargo/config.toml` and `scripts/codesign-run.sh`). `make run` also sets
`AGENT_TOOLBOX_BASE_ROOTFS`, the base image that instance disks are cloned from. The base
rootfs is rebuilt only when files in `vms/rootfs/base` change; `make clean-guest` removes all
guest artifacts. The supervisor and observer aren't part of it: every boot gets the current
ones from `target/guest/bin`, so after changing them `make guest-bin` (or `make run`) and a
reboot of the instance are enough. Instance disks are clones of the base taken at creation,
so after a base change that existing instances can't use, run `make clean-app`.

Development builds (`cargo run`, `make run`) use the kernel, base rootfs and guest binaries from `target/`
and don't update themselves; Preferences says so.

## Releases

Commits follow [Conventional Commits](https://www.conventionalcommits.org): `feat:` and `fix:`
commits make the changelog, `docs:`, `chore:`, `refactor:` and `ci:` don't.
`.github/workflows/release.yml` runs on every push to `main`:

- [release-please](https://github.com/googleapis/release-please) keeps a release PR with the
  changelog. Whenever it creates or updates that PR, the workflow publishes a **dev** build:
  prerelease `dev-<run number>`, version `<next version>-dev.<run number>`.
- Merging the release PR creates the `vX.Y.Z` release; the workflow adds the **stable** build to it.
- Each build is packaged, signed and notarized by [Velopack](https://velopack.io) (portable zip,
  full and delta update packages, and for stable releases a `.pkg` installer) and gets the guest
  kernel and base rootfs, built on an arm64 Linux runner, attached. Dev builds install from the
  portable zip, or from a stable installer by switching the channel in Preferences.
- Installed apps update themselves from these releases; the channel (stable or dev builds) is
  chosen in Preferences.

The signing job runs in the GitHub environment `release` with these secrets:

| Secret | Content |
|---|---|
| `MACOS_APP_CERT_P12` | Developer ID Application certificate with its private key, `.p12`, base64 |
| `MACOS_INSTALLER_CERT_P12` | Developer ID Installer certificate with its private key, `.p12`, base64 |
| `MACOS_CERT_PASSWORD` | Password of both `.p12` files |
| `ASC_KEY_ID`, `ASC_ISSUER_ID` | App Store Connect API key (role Developer), for notarization |
| `ASC_KEY_P8` | The API key's `.p8` file content |

The repository needs "Read and write permissions" and "Allow GitHub Actions to create and
approve pull requests" under Settings → Actions → General. No personal access token is needed.
