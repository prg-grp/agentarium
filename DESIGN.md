# DESIGN.md

## Purpose and status

Design a dark-only macOS desktop application for managing sandboxed AI agent instances.

The main tasks are configuring an instance, interacting with its terminal, and inspecting recorded activity. The interface must remain usable without knowledge of virtualization, Linux, protobuf, or SQLite.

This is a design specification. Required behavior follows the product brief in [ARCHITECTURE.md](ARCHITECTURE.md). The visual system below was agreed through mockups; screen-level details live in [design/](design/). The UI toolkit is still an open decision, so the specs describe appearance and behavior, not widgets of a particular framework.

The application is desktop only. No mobile or narrow layouts are needed. Design for a minimum window width of about 1100 px.

## How to use these documents

- Read this file for the visual system, vocabulary, and rules that apply everywhere.
- Read [design/app-shell.md](design/app-shell.md) for the window frame (sidebar, instance header, tabs) that most screens share.
- Read the screen file for the screen you are building. Each screen file lists its layout, states, exact copy, and behavior, and links back here instead of repeating tokens.

## Design principle

The application is an evidence instrument. Most of the window stays quiet and grey; color is spent on state and evidence only:

- Green means running.
- Amber means maintenance or a warning that needs attention.
- Rose means blocked, failed, or destructive.
- Diagonal hatching means "not observed" (a monitoring gap). It is never used decoratively.
- The selection blue marks focus, the selected item, and forward actions. It never carries status.

Every status color is paired with an icon and a word. Never rely on color alone.

## Visual system

### Color tokens

| Token | Value | Use |
|---|---|---|
| `bg` | `#101216` | Window background, main content |
| `panel` | `#171A20` | Sidebar, detail panes, live activity rail, dialogs, summary bars |
| `raise` | `#20252D` | Buttons, selected sidebar item, active segment, chips that are on |
| `well` | `#0C0E12` | Terminal background, text fields, argv blocks. The darkest surface |
| `line` | `#262C36` | Inner hairlines: row separators, section dividers |
| `line2` | `#343C49` | Control borders, window border, vertical separators in toolbars |
| `tx` | `#F0F3F7` | Primary text |
| `tx2` | `#AAB4C3` | Secondary text, inactive tabs, neutral results ("Succeeded") |
| `tx3` | `#7C8696` | Column headers, timestamps, hints, placeholders, inactive state |
| `acc` | `#8BA7FF` | Selection, focus, active tab underline, primary button fill, links |
| `ok` | `#78C99A` | Running states |
| `warn` | `#E8BC6A` | Maintenance mode, warnings, degraded monitoring, writable shares |
| `bad` | `#F08C92` | Blocked, failed, destructive actions |
| `hatch` | `#5A6475` | Monitoring gap hatching (45° stripes, 2 px line, 8 px period, ~28% opacity on rows) |

Derived values:

| Use | Value |
|---|---|
| Selected table row | `#1B2130` with a 2 px `acc` inset bar on the left |
| Popover / menu surface | `#1C2028`, highlighted item `#232A38` |
| Maintenance banner fill | `warn` at 12% opacity, bottom border `warn` at 35% |
| Warning notice fill | `warn` at 8% opacity |
| Pending-changes bar fill | `acc` at 8% opacity |
| Modal scrim | `#0A0B0E` (opaque mock) or black at ~45% |
| Text on `acc` fill | `#101216` |
| Text on `warn` fill | `#2A1E06` |
| Text on `bad` fill | `#2B0C0E` |
| Window traffic lights (inactive look in mockups) | `#3A414D` |

Check contrast in the rendered interface. `tx3` on `panel` is about 5:1 and is the lowest-contrast text allowed.

No gradients (other than the gap hatch), no drop shadows except focus rings and popovers, no glow, no animated status effects.

### Typography

- **UI text:** the system font (SF Pro). Two weights only: 400 regular and 500 medium.
- **Data text:** the system monospace (SF Mono) for terminal content, commands, argv, paths, IP addresses, domains, environment variable names and values, PIDs with process names, and precise timestamps. Do not use monospace for decorative labels.

| Role | Size / line height | Weight |
|---|---|---|
| Instance name in header, screen title ("New instance") | 22 / 28 | 500 |
| Dialog title, empty-state sentence, failure headline | 17 / 24–26 | 500 (empty state 400) |
| Section heading inside a screen | 15–16 / 20 | 500 |
| Body, table cells, buttons | 14 / 20 | 400, buttons 500 |
| Secondary descriptions, notices, detail rows | 13 / 20 | 400 |
| Status labels, column headers, metadata | 12 / 16 | 400 |
| Monospace data | 12.5 / 20–21 | 400 |

Sentence case everywhere. No all-caps labels. No mid-sentence bold or colored single words in headings.

### Spacing, shape, and density

- Window corner radius 12 px; dialogs 12 px; popovers 10 px; sidebar items and list highlights 8 px; buttons, fields, and segmented controls 7 px; chips are pill-shaped (13 px radius at 26 px height).
- Controls are 32 px tall (compact variants 26–28 px in toolbars and tab bars).
- Instance header padding 26 px vertical, 28 px horizontal. Content sections use 20–28 px side padding.
- Table rows: 10–12 px vertical padding, 1 px `line` separators, no zebra striping, no cards around rows.
- Layout is airy rather than dense: prefer fewer, larger rows over squeezing more data in. Dense lists are bordered rows, never a grid of cards.

### Icons

Mockups use Tabler outline icons. The final set depends on the UI toolkit; keep the meanings consistent:

| Meaning | Mockup icon |
|---|---|
| Running (sidebar) | `circle-dot` |
| Stopped (sidebar) | `circle` |
| Maintenance | `tool` |
| Startup failed / warning | `alert-triangle` |
| Draft instance (not created yet) | `pencil` |
| Virtual machine status | `server-2` |
| Agent status | `sparkles`; not started `player-pause`; exited `square-x` |
| Monitoring status | `heartbeat` |
| Monitoring gap | `cloud-off` |
| File / process / network / DNS / policy / lifecycle | `file`, `terminal-2`, `plug-connected`, `world-search`, `shield`, `power` |
| Blocked | `ban` |
| Failed operation | `x` or `circle-x` |
| Possible domains (ambiguous) | `arrows-split` |

Icon-only buttons need accessible labels.

## Buttons and emphasis

At most one filled button per view.

| Style | Look | Used for |
|---|---|---|
| Primary | `acc` fill, dark text | Forward actions: Create instance, Continue to …, Create and start, Start, Retry start |
| Neutral | `raise` fill, `line2` border | Stop, Restart now, Return to agent mode, Restart agent, Add folder, Create without starting |
| Ghost | No fill, `tx2` text | Restart in maintenance, Boot in maintenance, Back, Cancel, Reboot instance, Copy diagnostics |
| Danger ghost | No fill, `bad` text | Delete in the header |
| Mode-confirm | `warn` fill | Only the confirm button of the maintenance dialog |
| Danger-confirm | `bad` fill | Only the confirm button of the delete dialog |

Stop is deliberately neutral, not primary: a filled blue button always means "go ahead". Force stop is a secondary action reached from Stop (menu or follow-up), explained at the point of use.

## Status vocabulary

The Status tab always shows three separate readings ([design/status-tab.md](design/status-tab.md)); the instance header shows none. Never collapse them into one indicator.

| Reading | Values |
|---|---|
| Virtual machine | Running, Starting, Stopping, Stopped, Failed |
| Agent | Running, Starting, Not started, Exited, status *n*, Not running, Not running in maintenance, Unknown (status reports overdue) |
| Monitoring | Running, Degraded, Failed to start, Not running, Not installed (no collector in the guest yet), Unknown (status reports overdue) |

Boot mode is not a fourth reading. Maintenance is shown through the window-level amber treatment ([design/maintenance-session.md](design/maintenance-session.md)).

Sidebar items show one plain-language summary under the name: "Running", "Agent exited", "Maintenance, internet open", "Startup failed", "Stopped 2 days ago", "Not created yet".

## Evidence wording

Activity text states what was recorded, never what the agent intended.

Use:

- "Connection succeeded."
- "Blocked by rule: example.com."
- "Blocked: unlisted destination."
- "DNS lookup recorded; no related connection recorded."
- "Possible domains: a.example, b.example."
- "No domain recorded."
- "No matching activity recorded during this interval."
- "Monitoring gap overlaps this interval."
- "Activity in this interval is unknown."

Never display "The agent searched the web" or "The agent used model knowledge". Count labels are exact ("47 network connections", "2 blocked attempts"), never "searches". Do not invent full URLs from DNS or connection metadata; a URL may appear only where it is part of recorded argv.

## Sample data

Every mockup and design-time fixture shows the text "Sample data" in a quiet footer so it cannot be mistaken for implemented collection. Do not ship sample activity in the product; the empty state has no invented data.

## Writing

- Sentence case, plain verbs, no "please", no exclamation marks, no "successfully".
- A button names its action, and the action keeps its name through the flow ("Restart in maintenance" in the header, dialog title, and confirm button).
- Errors say what happened and what to do, beside the affected control. They never apologize and never show raw exception text in ordinary flows.
- Avoid transport and storage terms (vsock, protobuf, SQLite, eBPF) in ordinary flows. They may appear in diagnostics, the boot log, and copied diagnostics.

## Cross-cutting behavior

### Configuration and revisions

- Creation and editing use the same configuration model and the same section layouts.
- Saved changes apply on next boot. The UI names the running revision and the saved revision.
- Each run records its effective configuration. Old activity is always interpreted against the revision that applied at that time.

### Destructive actions and recovery

- Graceful Stop is prominent; Force stop is secondary and explains that unsaved guest writes may be lost.
- Deleting an instance lists the VM disk, configuration, and activity history being removed, and states that shared host folders and Keychain items remain.
- Recoverable failures appear beside the relevant control: missing shared folder, invalid launch command, guest service failure, unresolved domain, insufficient disk space. Entered settings are preserved after failure.
- Confirmation only for consequential actions: deleting an instance, force stop, entering maintenance. Routine navigation and edits stay direct.

### Credentials

Credential setup is separate from environment variables. Secrets are stored in macOS Keychain, masked in the UI, and never included in activity, configuration history, or copied diagnostics. Not built yet: for now the agent signs in inside the terminal, and environment variables are stored and passed as plain values. A secret mode for variables comes later.

### Terminal recording

Terminal recording is an unresolved product choice. Do not imply that terminal output is retained unless recording is enabled and working. The live session's scrollback is kept until the next start.

### Accessibility and interaction

- Keyboard-accessible navigation, visible focus (focus ring in `acc`), labeled icon buttons, adequate contrast.
- Support text scaling in the application and the terminal.
- Tables support selecting and copying values without opening a dialog; long commands and paths stay copyable.
- Pair status colors with icons and words.
- Respect reduced motion. Motion is only used in response to user actions (opening a pane, expanding a menu).

## Screens

| Screen | File | Covers |
|---|---|---|
| App shell | [design/app-shell.md](design/app-shell.md) | Sidebar, instance header, tabs, shared frame |
| Empty application | [design/empty-state.md](design/empty-state.md) | No instances yet |
| Create: harness | [design/create-harness.md](design/create-harness.md) | Creation flow, name, harness choice, provisioning script |
| Create: agent | [design/create-agent.md](design/create-agent.md) | Launch, sign-in, environment |
| Create: shared folders | [design/create-shared-folders.md](design/create-shared-folders.md) | Folder table, access levels, direct-write warning |
| Create: network | [design/create-network.md](design/create-network.md) | Default action, domain and address allow/deny lists, conflicts |
| Create: review | [design/create-review.md](design/create-review.md) | Read-back, resources, start choice, create |
| Running instance (terminal) | [design/terminal-running.md](design/terminal-running.md) | Agent terminal with live activity rail |
| Status tab | [design/status-tab.md](design/status-tab.md) | Readings, live CPU and memory meters, agent restarts |
| Activity timeline | [design/activity-timeline.md](design/activity-timeline.md) | Filters, counts, timeline, gap rows, event details |
| Activity network view | [design/activity-network.md](design/activity-network.md) | Destinations, blocked and ambiguous attribution |
| Startup failure | [design/startup-failure.md](design/startup-failure.md) | Boot stages, withheld agent, boot log |
| Agent exited, monitoring degraded | [design/agent-exited-degraded.md](design/agent-exited-degraded.md) | Exit bar, degraded notice, partial gap |
| Maintenance confirmation | [design/maintenance-confirmation.md](design/maintenance-confirmation.md) | Effects before entering maintenance |
| Delete confirmation | [design/delete-confirmation.md](design/delete-confirmation.md) | What is removed, what stays |
| Maintenance session | [design/maintenance-session.md](design/maintenance-session.md) | Persistent mode indication, root shell |
| Stopped instance with history | [design/stopped-history.md](design/stopped-history.md) | Start actions, run picker, retention boundary |
| Settings with pending changes | [design/settings-pending.md](design/settings-pending.md) | Restart-required bar, per-rule change marks |

## Shared compositions (flagged for the engineer)

These patterns appear on more than one screen. They are flagged here, per [AGENTS.md](AGENTS.md); the engineer decides whether to extract them.

| Composition | Appears in |
|---|---|
| Sidebar instance item (icon, name, summary line, selected bar) | All instance screens, creation |
| Instance header (name, agent, actions) | Terminal, Status, Activity, Settings, failure, maintenance, stopped |
| Tab bar with pending count and optional live-activity toggle | All instance screens |
| Live activity row (time, icon, target, result line) | Terminal, degraded, maintenance |
| Monitoring gap row (hatched) | Timeline, live activity rail |
| Event table row (six columns) | Timeline, stopped history |
| Detail pane with key/value rows | Timeline details, network details |
| Counts bar | Timeline, network view, stopped history |
| Segmented control | Folder access, network default action, Timeline / Network switch |
| Domain rule row (domain, scope, change mark, remove) | Create network, settings |
| Step indicator | All five creation steps |
| Confirmation dialog layout (title, intro, icon list, actions) | Maintenance and delete confirmations |
| Notice bar (icon, sentence, action) | Degraded monitoring, maintenance banner, pending changes |

## Open design decisions

- Subdomain matching is a per-rule setting ("Exact name" / "Includes subdomains") rather than wildcard syntax. Confirm this with users who write firewall rules.
- The Activity tab label: "Activity" (current) or "Observations".
- Whether "New instance" opens the creation flow directly (current) or first asks for a name.
- Whether clicking a row in the network view opens the same right-hand detail pane as the timeline (current) or a dedicated layout.
- The final icon set and how the system font and monospace are loaded depend on the UI toolkit.
- Terminal recording defaults.
