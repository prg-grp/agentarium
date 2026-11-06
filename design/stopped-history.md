# Stopped instance with history

A stopped instance with the Activity tab open and the run picker expanded. Frame: [app-shell.md](app-shell.md). Table and counts: [activity-timeline.md](activity-timeline.md).

## Layout

```
docs-site                                                       [Boot in maintenance] [▶ Start] │ Delete
Claude Code
──────────────────────────────────────────────────────────────────────────────────────────────────
Terminal   Logs   Status   Activity   Settings
──────────────────────────────────────────────────────────────────────────────────────────────────
[Timeline|Network]  [⟲ Run 23, Nov 2 at 09:12 ▴]  [⌕ Search paths, commands and addresses     ]
           ┌────────────────────────────────────────────────────────────┐
3,906 file │ Runs of docs-site                                          │  ✓ Monitoring complete
───────────│ ✓ Run 23, Nov 2, 09:12 to 11:40                    Stopped │ ─────────────────────
Time       │   Agent mode, configuration revision 4                     │
11:40:02   │ ⚒ Run 22, Nov 2, 08:51 to 09:10                    Stopped │
11:39:44   │   Maintenance, internet rules lifted                       │
11:39:12   │ ● Run 21, Oct 30, 14:03 to 18:27            Force stopped  │
           │   Agent mode, configuration revision 3, 2 monitoring gaps  │
           │ ⚠ Run 20, Oct 30, 13:58                     Startup failed │
           │   Agent mode, configuration revision 3                     │
           │ 📅 Choose a time range across runs                          │
           │ ────────────────────────────────────────────────────────── │
           │ History before Oct 1 was removed by the 30-day retention   │
           │ setting.                                                   │
           └────────────────────────────────────────────────────────────┘
```

## Header and Status tab

- Status tab: all three readings in `tx2` with `tx3` icons: "Stopped", "Not running", "Not running".
- Actions: "Boot in maintenance" (ghost, opens [maintenance-confirmation.md](maintenance-confirmation.md)), "Start" (primary, play icon), divider, "Delete".
- Sidebar: hollow circle in `tx3`, "Stopped 2 days ago".

## Activity while stopped

- Activity and Settings work normally. History stays available.
- Terminal tab while stopped: an empty well with "The instance is stopped. Start it to open the agent session." and the Start button. Do not show old terminal output unless recording is enabled and working.
- Counts bar for a complete run says "Monitoring complete for this run".

## Run picker

- Popover (`#1C2028`, 1 px `line2` border, 10 px radius, about 520 px wide) under the run button; the button shows an `acc` border while open.
- Heading "Runs of <instance>" in `tx3` 12 px.
- One row per run, newest first: mode icon (check for the selected one, tool in `warn` for maintenance, alert in `bad` for failed start), title "Run 23, Nov 2, 09:12 to 11:40", a second line with mode, configuration revision, and monitoring gaps if any, and the outcome on the right (Stopped, Force stopped, Startup failed in `bad`).
- Selected run highlighted with `#232A38`.
- Last row: "Choose a time range across runs" (calendar icon).
- Retention boundary at the bottom, separated by a `line` rule: "History before Oct 1 was removed by the 30-day retention setting." Retention gaps must never look like inactivity.

## Rules

- Viewing an old run shows the configuration and policy of that run's revision in event details.
- Maintenance runs are listed as their own runs and stay visible in history.
