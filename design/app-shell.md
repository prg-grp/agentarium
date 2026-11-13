# App shell

The window frame shared by every instance screen: sidebar, instance header, and tab bar. Tokens, button styles, and status vocabulary come from [DESIGN.md](../DESIGN.md).

## Layout

```
┌──────────────────┬──────────────────────────────────────────────────────────────────────┐
│ ● ● ●            │ api-refactor                     [Restart in maintenance] [Stop] │ Delete │
│                  │ Claude Code                                                          │
│ Instances    [+] │                                                                      │
│ ▌● api-refactor  ├──────────────────────────────────────────────────────────────────────┤
│    Running       │ Agent  Terminal  Activity  Status  Logs  Snapshots  Settings [Live a.]│
│  ⚒ scraper-lab   ├──────────────────────────────────────────────────────────────────────┤
│    Maintenance,  │                                                                      │
│    internet open │                      tab content                                     │
│  ⚠ infra-audit   │                                                                      │
│    Startup failed│                                                                      │
│  ○ docs-site     │                                                                      │
│    Stopped 2 d…  │                                                                      │
│                  │                                                                      │
│ ⚙ Preferences    │                                                                      │
└──────────────────┴──────────────────────────────────────────────────────────────────────┘
```

- Sidebar: 248 px wide, `panel` background, 1 px `line` right border.
- Main area: `bg` background, fills the rest.
- Minimum window width about 1100 px. Desktop only.

## Sidebar

- Top: window controls (unified title bar), then the "Instances" label in `tx2` / 500 with an icon-only "New instance" button (`+`, ghost, labeled for accessibility).
- Instance items: 18 px icon column, name (14 / 400; 500 when selected), summary line below (12 / 16).
  - Selected item: `raise` background, 8 px radius, 2 px inset bar on the left in `acc` (in `warn` when the selected instance is in maintenance).
  - Icon and summary color follow state: running `ok` + "Running"; maintenance `warn` + "Maintenance, internet open"; startup failure `bad` + "Startup failed"; agent exited `tx2` + "Agent exited"; stopped `tx3` + "Stopped 2 days ago"; draft `acc` pencil + "Not created yet".
- The instance list scrolls when it's taller than the sidebar; the "Instances" label and the Preferences row stay in place.
- Order: the user's. New instances go to the top. Each item shows a grip (`grip-vertical`, 14 px, `tx3`) at its right end on hover; dragging by it reorders the list, like the arguments in [create-agent.md](create-agent.md). The dragged item gets an `acc` 15 % background and a 1 px `acc` border; a 2 px `acc` line across the list marks where it lands, between items. Near the list's top or bottom edge, the drag scrolls the list. The order is stored and the selection stays on the same instance. The draft item has no grip, and nothing can be dropped above it.
- Instances set to "Start with Agentarium" start in agent mode when the app opens, once the guest image is ready.
- Bottom: "Preferences" with an adjustments icon, separated by a `line` top border. No account menu, because the manager has no application account. It opens [preferences.md](preferences.md) and shows "Update ready" when an update waits for a restart.
- Until the guest image is downloaded, [guest-download.md](guest-download.md) covers the whole window.
- With no instances, the list area shows "No instances yet" in `tx3` (see [empty-state.md](empty-state.md)).

## Instance header

Left to right:

1. Instance name (22 / 28, 500) with the agent name below in `tx2` ("Claude Code").
2. Flexible space.
3. Actions, right-aligned with 10 px gaps:
   - Running: "Restart in maintenance" (ghost, tool icon), "Stop" (neutral, stop icon).
   - Stopped: "Boot in maintenance" (ghost), "Start" (primary).
   - Maintenance session: "Stop" only; "Return to agent mode" lives in the maintenance banner.
   - Then a 1 px × 22 px `line2` divider and "Delete" (danger ghost, trash icon).

The header carries no status readings; the Virtual machine, Agent and Monitoring readings live in the Status tab ([status-tab.md](status-tab.md)). Maintenance is shown by the amber window treatment ([maintenance-session.md](maintenance-session.md)), not by a mode reading.

## Tab bar

- 44 px tall, 28 px gaps, 1 px `line` bottom border.
- Tabs: Agent, Terminal, Activity, Status, Logs, Snapshots, Settings. Active tab: `tx`, 500, 2 px underline in `acc` (in `warn` during maintenance). Inactive: `tx2`.
- Settings shows "2 pending" in `acc` 12 px when saved changes wait for the next boot.
- Agent shows the agent session; see [terminal-running.md](terminal-running.md).
- Terminal shows a login shell as `ubuntu` beside the agent, in the same terminal view, for looking around without ending the agent's session. It exists in agent mode only. The shell opens the first time the tab is shown during an agent boot and keeps running across tab switches until it exits or the VM stops. Header: "Shell as ubuntu" (`tx2`, 500); once the shell has ended, a "New shell" button (ghost, compact, plus icon) at the right end starts a fresh one. Exit bar texts: "The shell ended." after an exit; "The shell opens when the agent runs." before the boot is ready; in maintenance and provisioning boots, "The terminal is available while the agent runs. In maintenance mode, the Agent tab is a root shell." While the instance starts, stops or restarts, the view is covered as the Agent tab is. What the user does in this shell is not shown in Activity.
- Logs shows the instance's boot log in the same terminal view, read-only: kernel, systemd and supervisor output from boot to shutdown. Header: "Boot log" (`tx2`, 500), "Serial console, not the agent session" (`tx3`), and an "Open logs" button (ghost, compact, folder icon) that opens the instance's logs folder in Finder. No cursor; typing and pasting do nothing, while selecting, copying, scrolling, the scrollbar, the scroll shortcuts and "Jump to latest" work as in the Agent tab. The tab keeps the output until the next start; each boot's output is also kept for good as `<start>.log` in the logs folder, named by the boot's start as a unix timestamp.
- Status shows the readings and the live resource use; see [status-tab.md](status-tab.md).
- Snapshots lists the instance's snapshots and creates, restores and deletes them; see [snapshots.md](snapshots.md).
- On the Agent tab only, a "Live activity" toggle sits at the right end (ghost, compact, sidebar-right icon). Pressed state: `raise` background and `tx` text. It shows or hides the live activity rail; the terminal takes the full width when hidden. Remember the choice per instance.

## Removed on purpose

- No run timeline strip under the header (it was rejected as unnecessary; gaps are shown on the Activity timeline instead).
- No status bar under the terminal with shares, policy, or revision. That information belongs in Settings.
