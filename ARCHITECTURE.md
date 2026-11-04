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

The initial agent integration is Claude Code. The architecture must accommodate Codex and custom agents later without implementing those integrations in the first release.

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
| Boot modes | Normal agent mode and maintenance mode |
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

### Rust and Apple integration

Requirement: Rust owns the application’s main logic.

Proposed implementation: a narrow macOS integration layer using Rust bindings or a small Swift/Objective-C bridge. Keep Apple-specific types behind an interface covering VM configuration, lifecycle, shared directories, networking, and device connections.

Open decisions:

- Rust UI toolkit and terminal component.
- Direct bindings versus a native bridge.
- Minimum macOS version.
- Supported Mac architectures.
- Guest distribution, kernel, image production, and update process.
- Signing, distribution, and entitlement configuration.

Virtualization.framework requires the virtualization entitlement for the process using its APIs. Packaging must account for this requirement. [Apple entitlement documentation](https://developer.apple.com/documentation/virtualization/adding-the-virtualization-entitlement-to-your-project).

An Apple silicon first release with an arm64 Linux guest is a proposed scope reduction, not an established compatibility promise.

## Instance configuration and lifecycle

Persist an instance identifier, display name, agent type, VM resources, disk location, mount definitions, network policy, launch settings, environment configuration, and configuration revision.

Represent launch settings as executable plus argument vector, working directory, and environment. If shell-command execution is supported, make it explicit rather than silently interpreting every command through a shell.

Distinguish VM state, agent state, and monitoring health. A running VM can contain an exited agent or a failed collector.

Suggested VM states:

`Stopped -> Starting -> Running -> Stopping -> Stopped`

Failures must retain a useful error and indicate whether a VM process still exists. Record the boot mode separately.

Proposed initial editing behavior: save configuration changes for the next boot. Display the running revision and pending changes. Hot application of individual settings is deferred until its behavior is defined and verified.

Graceful stop should request guest shutdown. Force stop should be a separate action with a clear indication that unsaved guest writes may be lost.

## Agent and maintenance modes

### Agent mode

Normal startup must:

- Apply the instance’s configured mounts and network policy.
- Start guest services and monitoring.
- Launch the configured agent.
- Attach the terminal to its interactive session.

Proposed behavior: do not launch the agent until required policy enforcement and monitoring initialization succeed. Distinguish startup failure from degraded monitoring during a run.

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

The exact maintenance networking exception remains to be specified; “internet restrictions lifted” must not silently become access to every host service.

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

Deferred: copied workspaces, isolated change sets, diff review, and selective application back to the host.

## Network policy

### Required behavior

Users configure domain allow/deny rules during creation and later editing. The application resolves domains and applies the resulting access policy.

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

Bind each connection to the host-managed VM and current run. Do not trust an instance ID supplied in an arbitrary guest message as authority. Validate all guest input and avoid a generic host-command execution endpoint.

Use a guest PTY for agent and maintenance sessions. Boot diagnostics may use a separate serial console. Telemetry backpressure must not freeze terminal interaction.

## SQLite and local data

Requirement: SQLite stores local configuration and event history. A separate time-series service is unnecessary for the initial application.

Proposed logical records:

- Instances and configuration revisions.
- Runs and lifecycle events.
- Source events.
- DNS observations.
- Derived correlations and aggregate buckets.
- Collector health and known gaps.

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
