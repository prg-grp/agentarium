# App shell

The window frame shared by every instance screen: sidebar, instance header, and tab bar. Tokens, button styles, and status vocabulary come from [DESIGN.md](../DESIGN.md).

## Layout

```
┌──────────────────┬──────────────────────────────────────────────────────────────────────┐
│ ● ● ●            │ api-refactor       │ VM        │ Agent     │ Monitoring │ [Restart in  │
│                  │ Claude Code        │ Running   │ Running   │ Running    │  maintenance]│
│ Instances    [+] │                                                         [Stop] │ Delete │
│ ▌● api-refactor  ├──────────────────────────────────────────────────────────────────────┤
│    Running       │ Terminal   Logs   Activity   Settings 2 pending   [Live activity]    │
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
- Bottom: "Preferences" with an adjustments icon, separated by a `line` top border. No account menu, because the manager has no application account.
- With no instances, the list area shows "No instances yet" in `tx3` (see [empty-state.md](empty-state.md)).

## Instance header

Left to right:

1. Instance name (22 / 28, 500) with the agent name below in `tx2` ("Claude Code").
2. Three status readings, each separated by a 1 px `line` left border and 22 px padding: icon (18 px), label in `tx3` 12 / 16, value 14 / 20 in the state color. Readings: Virtual machine, Agent, Monitoring. Values are listed in [DESIGN.md](../DESIGN.md#status-vocabulary).
3. Flexible space.
4. Actions, right-aligned with 10 px gaps:
   - Running: "Restart in maintenance" (ghost, tool icon), "Stop" (neutral, stop icon).
   - Stopped: "Boot in maintenance" (ghost), "Start" (primary).
   - Maintenance session: "Stop" only; "Return to agent mode" lives in the maintenance banner.
   - Then a 1 px × 22 px `line2` divider and "Delete" (danger ghost, trash icon).

Do not add a separate mode reading. Maintenance is shown by the amber window treatment ([maintenance-session.md](maintenance-session.md)).

## Tab bar

- 44 px tall, 28 px gaps, 1 px `line` bottom border.
- Tabs: Terminal, Logs, Activity, Settings. Active tab: `tx`, 500, 2 px underline in `acc` (in `warn` during maintenance). Inactive: `tx2`.
- Settings shows "2 pending" in `acc` 12 px when saved changes wait for the next boot.
- Logs shows the instance's boot log in the same terminal view, read-only: kernel, systemd and supervisor output from boot to shutdown. Header: "Boot log" (`tx2`, 500) and "Serial console, not the agent session" (`tx3`). No cursor; typing and pasting do nothing, while selecting, copying, scrolling and "Jump to latest" work as in the Terminal tab. Output is kept until the next start.
- On the Terminal tab only, a "Live activity" toggle sits at the right end (ghost, compact, sidebar-right icon). Pressed state: `raise` background and `tx` text. It shows or hides the live activity rail; the terminal takes the full width when hidden. Remember the choice per instance.

## Removed on purpose

- No run timeline strip under the header (it was rejected as unnecessary; gaps are shown on the Activity timeline instead).
- No status bar under the terminal with shares, policy, or revision. That information belongs in Settings.
