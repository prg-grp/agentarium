# Agent Toolbox
This tool is a local-only, macOS-first Rust desktop manager for AI agents in persistent Virtualization.framework VMs. It supports Claude Code first. Other harnesses such as Codex will follow later.

The application features "agent instances". The user can create an instance which in the end configures a microVM for an agent. The microVM has its config such as CPU, RAM and other things but with meaningful defaults. The user can then configure other things, such as filesystem access, network access and environment variables, and which agent harness to use (claud code, others, custom).

Each instance has editable launch settings, environment variables, live host-directory mounts, DNS-resolved domain allow/deny policies, an interactive terminal, and activity history.

Normal boot starts the agent. One-time maintenance boot provides a privileged shell without the agent and temporarily lifts internet restrictions; ordinary reboot restores agent mode and policy.

Use protobuf over virtio-vsock for guest communication and SQLite for local configuration and events. Observe executions/argv, file operations, network activity, and monitoring gaps. Enforce access outside agent control.

UI toolkit, native bridge, collector, and network dataplane remain unresolved. Do not claim complete telemetry or exact hostname attribution. Defer other agents/platforms and copy/diff workflows.

ARCHITECTURE.md file describes the overall architecture of the application and its architectural decisions, whereas DESIGN.md describes how to implement UI / UX based items in this application. Don't just read the files each time, only if they may be necessary for the task at hand.

## Testing and Verification
If not explicitly requested in a prompt/session, this project does not need the addition of tests. So, no UI/magic tests shall be generated with the exception if specifically requested for a certain feature. This also accounts for general analysis. Checking if the code works shall be done via static analysis as well as local tools such as cargo check or cargo build (preferably cargo check). Along with "no tests": don't run the app to do some verification. Instruct the user how to verify the functionality manually. You are only allowed to run analyzers and build / checks as well as other static analytics tools and formatting tools.

Regarding style: keep the code lean. Compositions in the UI are currently kept where they are needed if they are only used in one particular screen. If there is a composition that is used in multiple screens, they shall be flagged and then the engineer may decide to extract them. But there must be no automatic creation of a lot of compositions and one time components. Same goes for helpers in the code. Reuse existing code as much as possible and don't create a load of helpers. Use sensible defaults when implementing functions and when to create a helper function for it. Simplicity and readability is more relevant than "cleverness".