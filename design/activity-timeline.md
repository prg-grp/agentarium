# Activity: all events

The Activity tab with the All events view selected. Frame: [app-shell.md](app-shell.md). The tab opens on the Commands view: [activity-commands.md](activity-commands.md). Network view: [activity-network.md](activity-network.md). Event fields come from [ARCHITECTURE.md](../ARCHITECTURE.md#event-content).

## Layout

```
[Timeline|Network]  [⟲ This run, started 13:41 ▾]  [⌕ Search paths, commands and addresses      ]
(Files)(Processes)(Network)(DNS)(Policy)(Lifecycle)(Health) │ (Agent)(System services)(Maintenance) │ (Any result ▾)
─────────────────────────────────────────────────────────────────────────── ┬──────────────────────
1,284 file operations │ 212 process executions │ 47 network connections │   │ Process execution  ×
2 blocked attempts                       ☁ Counts exclude 1 monitoring gap  │ 14:08:30.861 guest clock
─────────────────────────────────────────────────────────────────────────── │
Time          Type      Process    Action  Target                  Result   │ Arguments  Copy command
14:08:52.114  File      cargo 412  Write   /work/api/src/routes/…  Succeeded│ ┌────────────────────┐
14:08:31.902  Policy    curl 498   Block   104.20.67.143:443       ⊘ Blocked│ │0 curl              │
14:08:30.877  DNS       curl 498   Resolve pastebin.com            Succeeded│ │1 -sS               │
▌14:08:30.861 Process   bash 497   Execute curl -sS https://past…  Succeeded│ │2 https://pastebin… │
14:08:11.240  Network   cargo 402  Connect 151.101.130.137:443     Succeeded│ └────────────────────┘
14:08:10.502  Process   claude 211 Execute cargo add governor@0.6  Succeeded│ Working directory /work/api
14:07:41.009  File      rg 388     Open    /work/api/.env          × Permission denied
▨▨ 13:58:12 to 13:59:40  Monitoring gap. The collector restarted and 212   │ Process     curl, pid 498
▨▨ events were dropped. Activity in this interval is unknown.              │ Started by  bash 497
13:57:03.448  Lifecycle system     Start   Agent started in agent mode, …  │             from claude 211
Sample data                                                                 │ Attribution …
                                                                            │ Related …
```

- The list uses the full tab width. Selecting a row opens the detail page of what it names (the process, file, destination or DNS name), as described in [activity-commands.md](activity-commands.md#detail-page).

## Toolbar

Shared by all four views; see [activity-commands.md](activity-commands.md#toolbar).

- Segmented control "Commands | Files | Network | All events".
- Run picker and search field as in the Commands view.
- Filter chips (26 px pills, on = `raise` + `tx`, off = transparent + `tx2` with a `line2` border): "Show reads" (on by default) and "Failed or blocked only". In this view only, after a thin divider: "Processes", "Files", "Network" (all on by default).

## Counts bar

- `panel` background, 1 px `line` borders above and below, 13 px text, numbers in `tx` 500, items separated by 1 px `line` dividers.
- Exact labels: "commands", "file changes", "files read", "connections", "blocked" (blocked in `bad`, only when there is any).
- On the right, the monitoring of the run: "Monitoring complete for this run" (`tx3`), or in `warn` "N events lost; activity is incomplete", "Monitoring restarted N times; activity may be incomplete", "The observer didn't report; activity in this run is unknown". Zero events with healthy collection and missing events during an outage must read differently.

## Table

Columns: Time (100 px, monospace with milliseconds, `tx3`), Type (64 px), Target (flexible, monospace, with the action as a second line), Process (150 px, `name pid`), Result (220 px, right-aligned).

- Rows: program starts (and refused starts), privilege changes, file accesses (reads only with "Show reads"), guest connections with the Mac's outcome, connections seen on the Mac only, and DNS lookups. At most the newest 2,000 matching rows; the note under the list says "Showing the newest 2,000 of N events."

- Newest first. Header row in `tx3` 12 px.
- Result: "Succeeded" in `tx2` (no green; green is reserved for running state); "Blocked" in `bad` with ban icon; failures in `bad` with an x icon and the specific reason ("Permission denied"); "Unknown" in `tx3`.
- Selected row: `#1B2130` background with a 2 px `acc` left bar.
- Lifecycle rows use `system` in `tx3` as the process.
- Rows are 44 px high, all alike. Nothing is copyable yet; long targets show fully on the detail page.

### Monitoring gap row

- Full-width row, placed at the gap's start time in the ordering.
- Background: 135° hatching in `hatch` at ~28% opacity (2 px stripe, 8 px period).
- Cloud-off icon, the interval in monospace `tx`, then the explanation in `tx2` 13 px: "Monitoring gap. The collector restarted and 212 events were dropped. Activity in this interval is unknown."
- Partial gaps name what is missing: "File events incomplete. 1,040 dropped." (see [agent-exited-degraded.md](agent-exited-degraded.md)).

## Detail page

A row opens the page of what it names; see [activity-commands.md](activity-commands.md#detail-page). Uncertainty is stated there (attribution, truncation, loss) rather than hidden.

## Historical use

The same screen works for stopped instances and earlier runs. Each run shows the policy and configuration that applied at that time.

## Not built yet

- Monitoring gap rows in the list: the counts bar states lost events and observer restarts instead.
- Policy and configuration revision rows on the detail page, the "System services" source, Lifecycle and Health chips, and copying.
