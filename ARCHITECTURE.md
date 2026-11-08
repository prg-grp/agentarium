# Product and architecture brief

## Status and terminology

This document specifies a planned application. It does not establish that any feature has been implemented.

- **Requirement:** an agreed product or architecture decision.
- **Proposed implementation:** a concrete starting point that requires validation.
- **Open decision:** an unresolved choice.
- **Deferred:** outside the initial release.

An **agent instance** is a persistent virtual machine with its own agent configuration, filesystem, access policies, terminal, and activity history. A **run** is one boot of that instance.

## Product purpose

Build a local macOS desktop application for running AI coding agents inside isolated virtual machines while showing their observable activity.

Users should be able to configure an instance, start it, interact with its agent through a terminal, and inspect its file access, process execution, and network activity. The application should support ordinary development work and provide records useful for later behavioral analysis.

The initial agent integration is Claude Code. New instances are created from presets compiled into the app (Claude Code, Codex, Gemini CLI, OpenCode) or as a custom agent.

## Agreed requirements

| Area | Requirement |
|---|---|
| Host platform | macOS first |
| Implementation | Rust desktop application |
| Virtualization | Apple Virtualization.framework |
| Instances | Persistent, independently configured VMs |
| Agent integration | Claude Code initially |
| Configuration | Launch command, arguments, working directory, mounts, environment variables, network policy |
| File access | Live access to explicitly shared host directories |
| Network access | Configurable domain allow/deny policies using DNS resolution |
| Boot modes | Normal agent mode, maintenance mode, and provision mode |
| Interaction | Embedded interactive terminal |
| Observability | Live and historical activity for each instance |
| Guest communication | Protobuf messages over virtio-vsock |
| Persistence | Local SQLite database |
| Application service | Local-only operation; no application account or hosted backend |
| Appearance | Dark-only interface with a consistent visual system |

Local-only describes the manager and its storage. Agents may contact external AI providers and other permitted internet services. Provider credentials or subscriptions remain separate from the application.

## Initial user workflow

1. Create an agent instance and select Claude Code.
2. Configure its launch settings, shared directories, environment variables, and network rules.
3. Provision and boot the VM.
4. Interact with the agent in its terminal.
5. Inspect current activity and previous runs.
6. Stop the instance while preserving its disk and configuration.
7. Use maintenance mode when software installation or guest customization is needed.

Creation and later editing must expose the same configuration model. Record the effective configuration for every run so historical events remain interpretable after settings change.

## Platform and component boundaries

### macOS host

The host application owns:

- Instance configuration and lifecycle.
- Virtualization.framework integration.
- Host-directory sharing.
- Network policy enforcement and host-side network observations.
- Terminal presentation and input routing.
- Event ingestion, persistence, aggregation, and dashboard queries.
- Local credential references and application settings.

Apple exposes directory sharing through virtio-fs and guest communication through virtio sockets. These provide the relevant primitives; guest setup and the application protocol remain application responsibilities. [Apple directory-sharing documentation](https://developer.apple.com/documentation/virtualization/vzvirtiofilesystemdeviceconfiguration), [Apple virtio socket documentation](https://developer.apple.com/documentation/virtualization/vzvirtiosocketdeviceconfiguration).

### Guest

Proposed implementation: a managed Linux image containing:

- A boot supervisor.
- A guest service for control, telemetry, and terminal sessions.
- A privileged collector.
- The agent and its dependencies.
- A persistent writable disk for installed software and instance state.

Normal agent execution should use an unprivileged guest user. The privileged service should expose only the operations needed to manage that instance.

Linux is the proposed guest operating system. Later Linux host support is a separate product target.

Decided:

- One harness-agnostic base root filesystem (`vms/rootfs/base`, Ubuntu 24.04 arm64) is built as a sparse raw ext4 image at its full virtual size of 64 GiB. Inode tables and journal stay unwritten (mke2fs lazy init, guest mounts root with `noinit_itable`), so the image only takes the space of its contents. There is no resize step.
- Each instance disk is an APFS `clonefile` of the base: created instantly, sharing the base's blocks until the guest writes, and growing on the Mac only by what the guest writes. Without a clone (another volume) creation fails; there is no copy fallback.
- The agent harness is not part of the base. Each instance's provisioning script installs it in provision mode (see below). The base does include Node.js LTS with npm (NodeSource), which the Codex, Gemini CLI and OpenCode presets install from.
- During development, `make run` names the base image with `AGENT_TOOLBOX_BASE_ROOTFS`. How a released app obtains the kernel and base image (e.g. a pinned, downloaded guest bundle) remains open.
- The guest supervisor (`agentarium-supervisor`, crate `guest_supervisor`) is the boot authority in the guest. It is a static Rust binary for `aarch64-unknown-linux-musl`, cross-compiled on the Mac and baked into the base rootfs as a systemd service. No login runs on any console; nothing runs on the session console until the app sends Start over the command plane (see [Protobuf over vsock](#protobuf-over-vsock)). Each mode gives access to exactly one thing: the instance's launch command (agent), a root shell (maintenance), or the provisioning script (provision). The boot mode is no longer on the kernel command line. In agent mode the agent runs in a cgroup of its own below the supervisor's (systemd delegates the cpu and memory controllers with `Delegate=` and starts the supervisor in its own subgroup with `DelegateSubgroup=`), so its CPU and memory use include every process it starts.
- The guest has two virtio consoles. `hvc1` carries only the session (the app's Terminal tab, the one interactive terminal). `hvc0` is the kernel console: kernel, systemd (boot and shutdown) and the supervisor's logs, shown read-only in the app's Logs tab, where only selecting and copying work. `hvc0` also stays the path for a later rescue console.
- The supervisor logs with `tracing` to stdout; systemd sends it to the journal and to `hvc0` (`StandardOutput=journal+console`). Logs name programs, users, directories and environment variable names, but never argument or variable values or the provisioning script. Lines the user needs in the session (an exit and restart, a start failure, the provisioning result) are also written to `hvc1`.
- The supervisor opens `hvc1` anew for each session: when a session leader exits, the kernel hangs the console up. It restores the console's boot settings, drops pending input, and applies the latest size before each session.
- Disks are cloned once, so instances created from an older base keep the supervisor they were created with; a protocol mismatch fails the boot.
- Follow-up: a rescue path for a broken supervisor (for example a kernel flag that starts a plain root console without it). Root in maintenance mode can break the supervisor on the instance's disk, and without it the instance can't boot into any mode.

### Rust and Apple integration

Requirement: Rust owns the application’s main logic.

Proposed implementation: a narrow macOS integration layer using Rust bindings or a small Swift/Objective-C bridge. Keep Apple-specific types behind an interface covering VM configuration, lifecycle, shared directories, networking, and device connections.

Open decisions:

- Rust UI toolkit and terminal component.
- Direct bindings versus a native bridge.
- Minimum macOS version.
- Supported Mac architectures.
- Distribution and update process of the kernel and base image.
- Signing, distribution, and entitlement configuration.

Virtualization.framework requires the virtualization entitlement for the process using its APIs. Packaging must account for this requirement. [Apple entitlement documentation](https://developer.apple.com/documentation/virtualization/adding-the-virtualization-entitlement-to-your-project).

An Apple silicon first release with an arm64 Linux guest is a proposed scope reduction, not an established compatibility promise.

## Instance configuration and lifecycle

Persist an instance identifier, display name, agent type, VM resources, disk location, mount definitions, network policy, launch settings, environment configuration, provisioning script, and configuration revision.

Represent launch settings as executable plus argument vector, working directory, and environment. If shell-command execution is supported, make it explicit rather than silently interpreting every command through a shell.

Distinguish VM state, agent state, and monitoring health. A running VM can contain an exited agent or a failed collector.

Suggested VM states:

`Stopped -> Starting -> Running -> Stopping -> Stopped`

Failures must retain a useful error and indicate whether a VM process still exists. Record the boot mode separately.

Proposed initial editing behavior: save configuration changes for the next boot. Display the running revision and pending changes. Hot application of individual settings is deferred until its behavior is defined and verified.

Graceful stop should request guest shutdown. Force stop should be a separate action with a clear indication that unsaved guest writes may be lost.

Decided: Stop sends `Stop` over the command plane; the supervisor powers the guest off with systemd. Before the supervisor has connected, the app presses Virtualization.framework's power button instead. Either way the VM is stopped hard after 20 seconds. During the shutdown, systemd ends the session with the supervisor's unit: `SIGTERM`, right away `SIGHUP` (interactive shells ignore `SIGTERM`), and `SIGKILL` after 5 seconds, so the guest powers off well within that limit.

## Agent and maintenance modes

### Agent mode

Normal startup must:

- Apply the instance’s configured mounts and network policy.
- Start guest services and monitoring.
- Launch the configured agent.
- Attach the terminal to its interactive session.

Proposed behavior: do not launch the agent until required policy enforcement and monitoring initialization succeed. Distinguish startup failure from degraded monitoring during a run.

Decided: the app sends the launch settings of the boot's revision (command, arguments, working directory) in Start, together with its environment variables and shared folders, which maintenance mode gets too. The supervisor mounts the shared folders first (see [Live host-directory access](#live-host-directory-access)), then runs the command directly, without a shell, as the unprivileged `ubuntu` user on the console, and starts it again two seconds after it exits. It never falls back to a shell. The supervisor has no default command: an empty command starts nothing.

### Maintenance mode

The user must be able to boot the same persistent VM without launching the agent, obtain a privileged guest shell, and install or modify software.

Maintenance mode temporarily lifts the instance’s configured internet restrictions, as agreed. It must be visibly different from agent mode and recorded in the run history.

Maintenance boot is a one-time selection. The following ordinary reboot returns to agent mode and restores its configured network policy.

Keep monitoring active where possible. Clearly report collector changes or missing coverage. Root access can alter guest services, including the collector.

Proposed safeguards:

- Show the active mounts before entering maintenance mode.
- Keep host-service and private-network access restricted even when internet restrictions are lifted.
- Preserve host-side network enforcement outside the guest.
- Do not silently start the agent when maintenance preparation completes.

Decided: the maintenance networking exception is the open policy (default allow, no rules). The Mac and the local network stay unreachable, so “internet restrictions lifted” never becomes access to host services (see [Decided policy](#decided-policy)).

### Provision mode

The provisioning script installs and sets up the agent harness. It is part of the instance configuration, stored per revision, and comes from the preset chosen at creation (`vms/provision/`); only a custom agent's script is written by the user.

Decided:

- Starting an instance whose disk has not been provisioned successfully boots provision mode first. On success the instance is marked provisioned and boots in agent mode; on failure it stays unprovisioned, the script's output stays visible, and the next start provisions again. Scripts must therefore be safe to run again.
- The app sends the script in Start. The supervisor runs it as root with its output on the console, reports the exit code (`ProvisionFinished`), and powers off. No login runs in provision mode.
- Provisioning stays a separate one-time boot rather than a step of the agent boot: the agent boot starts fresh with the network policy, and nothing the script started (with open internet) carries over. The app's `provisioned_at` is the authority; there is no marker in the guest.
- Provision mode gets the same internet exception as maintenance mode.
- The harness is chosen at creation from a preset (`app/src/preset.rs`): Claude Code, Codex, Gemini CLI, OpenCode, or a custom agent. A preset defines the launch command (without arguments), the provisioning script (`vms/provision/`, compiled in with `include_str!`) and the initial network rules. Creating the instance copies them into revision 1; the preset itself isn't stored, so later app versions don't change existing instances. During creation a preset's command and script can't be edited; its network rules are only a starting point. Later, Settings edits the command like every other setting; the provisioning script only until provisioning has succeeded. A custom agent starts from the template `vms/provision/custom.sh`, which the user fills in; an unchanged template can be saved but not started.
- A new instance starts right after creation unless the user opts out, so its first boot provisions it.

## Live host-directory access

Requirement: explicitly selected host directories are shared directly with the guest. Guest modifications to writable shares affect the original host files immediately.

Each mount needs:

- Host directory.
- Guest mount path.
- Read-only or read-write access.
- Enabled state.

The guest image must mount the configured shares during boot. Validate missing directories, duplicate guest paths, access failures, and invalid configuration before agent startup.

Show the direct-write consequence during creation and editing. This release provides no review-before-apply workflow, automatic rollback, or protection from an authorized agent deleting files inside a writable share.

Proposed defaults:

- Read-only access until the user selects write access.
- No implicit sharing of the home directory, credential stores, or unrelated projects.
- Stable host-to-guest path mapping for activity display.

Decided:

- All enabled folders of a boot share one virtio-fs device (tag `agentarium-shares`) with a `VZMultipleDirectoryShare`; each folder is a named directory in it. Read-only folders are read-only `VZSharedDirectory`s, so the Mac enforces the access and guest root can't lift it.
- The app checks the folders before the boot (each must exist and be readable, guest paths must be absolute, without `..`, and unique) and fails the start with the reason otherwise.
- `Start` names each folder and its guest path. The supervisor mounts the share at `/run/agentarium/shares` (root only), then bind-mounts each folder to its guest path, parents first, and remounts read-only folders read-only. It creates missing mount points owned by `ubuntu` and refuses a path through a symbolic link. The mounts last for the boot only; nothing is written to fstab.
- Agent and maintenance boots get the folders; provision boots don't, as they run with open internet.
- If a folder can't be mounted, nothing starts: the supervisor logs the error, shows it on the session console, sends `StartFailed`, and powers off. The app shows the reason as a startup failure.
- Changing the folders of a running instance applies on the next boot.

Deferred: copied workspaces, isolated change sets, diff review, and selective application back to the host.

## Network policy

### Required behavior

Users configure allow/deny rules during creation and later editing. A rule targets either a domain (exact name, or including subdomains) or an IPv4/IPv6 address range in CIDR form; a single address is a /32 or /128. The application resolves domains and applies the resulting access policy; address rules match destinations directly. Rules are stored per revision in `network_rules` and enforced on the Mac (see [Decided policy](#decided-policy)).

Policy enforcement must sit outside the agent’s control. Guest firewall configuration alone cannot provide that boundary when maintenance grants guest root access.

Record policy decisions separately from observed connection outcomes. An allowed connection may fail; a DNS lookup may never lead to a connection.

### Proposed policy contract

Define these semantics before implementation:

- Default action: allow or deny unmatched traffic.
- Deny precedence when rules overlap.
- Exact names versus subdomain patterns.
- DNS refresh, TTL expiry, CNAME handling, and resolution failure.
- IPv4 and IPv6 behavior.
- Direct IP connections and unclassified destinations.
- Host, loopback, link-local, and private-network destinations.
- Existing connections when policy changes.

Proposed initial behavior: deny overrides allow; restrictive mode denies unclassified destinations; rule and DNS failures do not silently grant access. Evaluate private-network destinations separately from domain rules.

### Decided policy

Enforced in `app/src/policy.rs`, called by the VM's userspace network stack (`net`) for every new TCP connection and UDP flow, and for every DNS query. The policy is fixed for a boot, as the boot freezes its revision; changed rules apply on the next boot.

- **Destinations,** decided when a TCP connection or UDP flow opens:
  1. The Mac and the local network are never reachable, in any mode and whatever the rules say: loopback, private (10/8, 172.16/12, 192.168/16, which includes the guest's gateway), link-local, shared 100.64/10 (carrier-grade NAT, VPNs such as Tailscale), multicast, 0/8 and 240/4, and their IPv6 counterparts (loopback, fc00::/7, fe80::/10, multicast, IPv4-mapped).
  2. The destination's names are the names whose DNS answers returned its address and haven't expired.
  3. A deny rule that matches the address (CIDR) or one of its names blocks it. Deny wins, also when an address has several names and only one of them is denied.
  4. Otherwise an allow rule that matches the address or one of its names allows it.
  5. Otherwise the default action applies. An address without a recorded name is decided by address rules and the default only.
- **DNS:** every UDP datagram to port 53, to the gateway or any other address, goes to the Mac's nameserver, so another resolver can't skip the check. Each query is decided by its name with steps 3 to 5; a refused query gets a `REFUSED` answer, an unreadable one `FORMERR`. Refusing stops lookups (and DNS-based exfiltration) of names the policy doesn't allow.
- **Answers:** before the guest gets an answer, its A and AAAA records are recorded with the queried name and the names of the CNAME chain. An entry lasts for its TTL (at most one hour) plus five minutes, as programs keep using an address a little beyond its TTL. Expired entries are dropped as new answers arrive.
- Blocked connection attempts are logged on the app's standard error. Recording policy decisions as events follows with the activity store.
- Maintenance and provision boots use the open policy: default allow and no rules, so only step 1 applies; their DNS is forwarded unchecked but still recorded.
- Limitations: DNS over TCP to the gateway is blocked by step 1, so a truncated UDP answer can't be retried over TCP. Encrypted DNS (DoH, DoT) and hard-coded addresses bypass the name association: such an address has no names, so under default allow only address rules can block it. A blocked TCP connection is accepted by the stack and closed right away, rather than refused.

### Domain-policy limitation

DNS-derived IP filtering controls destination addresses associated with names. It cannot reliably establish the hostname intended by every connection. Shared hosting, CDNs, alternate DNS, encrypted DNS, and address reuse create ambiguity.

Do not present an IP-derived decision as proof that a request targeted a particular hostname. Store the DNS evidence and its time interval. Display ambiguous associations as candidates.

Strict application-layer hostname restrictions would require additional mediation and a defined protocol scope. That is an open architecture decision. TLS interception is not an agreed requirement.

### Enforcement implementation

Open decision: the host networking dataplane.

A candidate is `VZFileHandleNetworkDeviceAttachment`, which exposes raw network frames through a datagram socket. It requires an application-managed networking implementation; it is not a ready-made domain firewall. [Apple network attachment documentation](https://developer.apple.com/documentation/virtualization/vzfilehandlenetworkdeviceattachment).

Validate routing, DNS mediation, TCP/UDP support, IPv6, and bypass resistance before selecting this approach. Do not assume a default NAT attachment supplies the required filtering.

## Per-instance observability

### Required coverage

The dashboard must provide live and historical observations of:

- Process execution, including executable and arguments.
- File access and modification.
- Network activity and policy blocks.
- DNS observations and derived domain associations.
- Lifecycle events, boot mode, configuration revision, and monitoring health.

Scope collection to the instance and distinguish agent descendants, maintenance activity, and guest services where attribution is available.

### Collector choice

Open decision: Linux audit, custom eBPF collection, or a combination.

Custom eBPF is a candidate because the managed guest can provide a controlled kernel and targeted hooks. It is not an implemented or settled choice.

Before committing, establish coverage for execution arguments, file operations, failed operations, process identity, network attribution, and event loss. An open event does not prove bytes were read; a read does not prove the model used the content.

No collector should claim complete capture without a defined coverage model. Buffer exhaustion can lose events; Linux documents failed BPF ring-buffer reservations when capacity is unavailable. [Linux BPF ring-buffer documentation](https://www.kernel.org/doc./html/latest/bpf/ringbuf.html).

### Event content

Proposed common envelope:

- Schema version and event type.
- Instance ID, run ID, producer ID, and producer sequence number.
- Guest monotonic timestamp and host receipt timestamp.
- Guest wall time when available.
- Process identity, including PID plus a reuse-resistant start identifier.
- Operation result and error.
- Source, attribution quality, and truncation or loss indicators.

Type-specific content:

| Event | Useful fields |
|---|---|
| Execution | Executable, argv, working directory, user, parent identity, success/failure |
| File | Operation, guest path, mapped host path where available, result, bytes where captured |
| Network | Protocol, endpoints, attempt/result, bytes where available, process attribution |
| DNS | Query, answer, CNAME chain, TTL, resolver, observed time |
| Policy | Action, matching rule, policy revision, decision reason |
| Health | Collector status, lost events, transport backlog, storage failures |

Treat argv, paths, and terminal output as sensitive. Full argument capture is a functional target, subject to explicit limits and possible secret exposure. Do not collect all process environments or file contents by default.

### Interpretation limits

The application can show that a process contacted a destination. It cannot infer the agent’s reasoning from that observation.

For the question “Did Claude search online?”, show evidence such as DNS queries, connections, process attribution, and available application events. A provider connection may carry inference traffic rather than web browsing. Provider-side search may happen without a separate guest connection to the retrieved website.

Use statements such as “No matching network activity recorded in this interval,” with coverage status. Do not conclude that the agent relied only on model knowledge.

## Protobuf over vsock

Requirement: use protobuf messages over virtio-vsock for guest-host communication.

Proposed protocol:

- A versioned handshake with capability negotiation.
- Length-delimited frames and bounded message sizes.
- Separate logical channels for control, telemetry, and terminal I/O.
- Request IDs, responses, timeouts, and structured errors.
- Sequence numbers, bounded buffering, reconnection, and deduplication.
- Explicit loss and truncation reporting.
- Terminal input, output, resize, and session-close messages.

Protobuf supplies serialization; framing, acknowledgments, authorization, and recovery are application responsibilities.

Decided (first version, crate `command_plane`, `command_plane/proto/command_plane.proto`):

- The app connects to the supervisor on vsock port 700, retrying every 250 ms while the guest boots, and again if the connection closes. Only root can listen on ports up to 1023 in a Linux guest, so the agent user can't pose as the supervisor.
- Frames are a big-endian u32 length followed by the encoded message, at most 1 MiB. Both sides use blocking I/O on their own threads.
- Messages: `Hello` (guest, with the protocol version) → `Start` (host: mode, plus launch settings in agent mode or the script in provision mode, and environment variables and shared folders in agent and maintenance mode; only the first per boot counts) → `ProvisionFinished` or `StartFailed` (guest, when the shared folders couldn't be mounted), `Resize` (host: the Terminal view's columns and rows, sent after Start and on every change; the supervisor applies it to `hvc1`) and `Stop` (host). Without a Hello within 60 seconds of the VM's start, the app stops the VM and reports the failure.
- In agent mode only, the supervisor sends `Status` every 2 seconds (`STATUS_INTERVAL`) and at once when the agent starts or exits: CPU time of the VM and of the agent's cgroup as running totals (the app computes the share of all vCPUs from two reports at least a second apart), memory in use (the VM's MemTotal minus MemAvailable; the agent's cgroup without inactive file cache), the agent's out-of-memory kills, whether the agent runs, its restarts and last exit, and the collector state (always `NOT_INSTALLED` until a collector exists). The app keeps only the latest report per running instance and stores nothing; it shows them in the Status tab. Without a report for 10 seconds, the app shows the agent and monitoring as unknown until the next one arrives.
- Terminal I/O runs on its own serial console (`hvc1`), apart from the boot output on `hvc0`. A guest PTY with its own vsock stream is only needed once an instance runs several sessions at once; separate channels, request IDs, and telemetry follow.

Bind each connection to the host-managed VM and current run. Do not trust an instance ID supplied in an arbitrary guest message as authority. Validate all guest input and avoid a generic host-command execution endpoint.

Boot diagnostics use a separate serial console (decided, see above). A guest PTY for sessions remains the option for several sessions per instance. Telemetry backpressure must not freeze terminal interaction.

## SQLite and local data

Requirement: SQLite stores local configuration and event history. A separate time-series service is unnecessary for the initial application.

Proposed logical records:

- Instances and configuration revisions.
- Runs and lifecycle events.
- Source events.
- DNS observations.
- Derived correlations and aggregate buckets.
- Collector health and known gaps.

Decided:

- The app's files live in `~/Library/Application Support/com.programming-group.agentarium/`: the database `agentarium.sqlite`, and per instance `instances/<id>/` with its root disk `root.ext4` and provisioning folder `provision/`.
- The database is accessed with `rusqlite` (bundled SQLite) on the main thread.
- Any number of instances can run at once. Each running VM operates on its own serial dispatch queue and is driven by its own controller thread, which starts it, watches its state and stops it (with a hard stop after a timeout). Controllers report state changes and the console to the main thread, which keeps database access, the sidebar and two terminals per instance (session and boot log); the selected instance's terminal for the active tab is shown. Quitting asks every running VM to shut down and exits once all have stopped.
- `rusqlite_migration` upgrades the schema on start, tracked in `PRAGMA user_version`. Migrations are append-only SQL files; a shipped migration is never edited, reordered or deleted. An existing database is copied to `agentarium.sqlite.v<version>.bak` before an upgrade. A database newer than the app is refused.
- Instance configuration is stored as revisions. The latest revision is editable while pending; the first boot that uses it freezes it. List-valued settings (arguments, environment, mounts, network rules) are JSON columns.
- Launch settings (command, arguments, working directory, environment) and mounts are applied at boot through `Start`. Network rules are applied on the Mac by the VM's network stack for agent boots.

Preserve received source events separately from later enrichment. Enrichment must retain provenance and remain reproducible; do not overwrite the original destination with a guessed domain.

Index primarily by instance, run, time, event type, and process identity. Batch ingestion and use a bounded database writer. Evaluate WAL mode against expected concurrency.

Define retention limits and disk-full behavior before release. Deleting old data must create a visible retention boundary. Collector outages, dropped events, and persistence failures must remain distinguishable from inactivity.

Store large terminal recordings separately if needed, with SQLite metadata. Terminal-recording defaults are open. Credentials must not appear in routine logs, configuration history, or exports; proposed storage is macOS Keychain with configuration references.

## Trust boundaries

The VM separates the agent from host resources that have not been shared. Writable mounts intentionally grant authority over their contents. Allowed network access can transmit data.

Guest telemetry is evidence supplied by the guest, not a tamper-proof audit trail against guest root or kernel compromise. Host-side observations provide a separate source with their own attribution limits.

Treat terminal escape sequences, event strings, paths, and guest messages as untrusted input. Keep host file operations constrained to explicit user actions and approved share configuration.

## Initial delivery criteria

The first release is complete when:

- A user can create, edit, start, stop, and reopen a persistent Claude Code instance.
- Agent mode provides an interactive terminal.
- Maintenance mode provides a privileged shell, preserves installed changes, records its network exception, and returns to agent mode on ordinary reboot.
- Live mounts honor configured access and expose direct-write behavior accurately.
- Network rules produce explainable decisions under documented DNS/IP semantics.
- Per-instance history survives application restart and remains tied to the correct run and configuration.
- The dashboard distinguishes attempted, blocked, successful, failed, and unattributed activity.
- Collector loss, disconnects, and storage failures are visible.
- One instance’s configuration and event stream cannot be attributed to another.
- The manager requires no application account or hosted service.

## Deferred scope

- Windows and Linux host applications.
- Codex and custom-agent integrations.
- Copied workspaces with diff review and selective apply.
- Cloud synchronization, collaboration, and remote management.
- TLS interception or decrypted request inspection.
- Claims about model reasoning or semantic use of observed data.
