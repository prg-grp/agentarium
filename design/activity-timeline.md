# Activity: timeline

The Activity tab with the Timeline view selected and an event open in the detail pane. Frame: [app-shell.md](app-shell.md). Network view: [activity-network.md](activity-network.md). Event fields come from [ARCHITECTURE.md](../ARCHITECTURE.md#event-content).

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

- Main column (flexible) + detail pane (380 px, `panel`, 1 px `line` left border). The pane closes with its × button; the table then fills the width.

## Toolbar

- Segmented control "Timeline | Network".
- Run picker: ghost button with history icon showing the selected range ("This run, started 13:41"). The menu lists runs and a custom time range; see [stopped-history.md](stopped-history.md#run-picker).
- Search field on `well` with a search icon: "Search paths, commands and addresses".
- Filter chips (26 px pills, on = `raise` + `tx`, off = transparent + `tx2`), in three groups separated by thin vertical dividers:
  1. Event type: Files, Processes, Network, DNS, Policy, Lifecycle, Health (each with its type icon).
  2. Source: Agent, System services, Maintenance. System services are off by default.
  3. Result dropdown: Any result, Succeeded, Failed, Blocked, Unknown.
- Process and target filters come from the search field and from "Filter to this process" actions in the detail pane.

## Counts bar

- `panel` background, 1 px `line` borders above and below, 13 px text, numbers in `tx` 500, items separated by 1 px `line` dividers.
- Exact labels: "file operations", "process executions", "network connections", "blocked attempts" (blocked in `bad`).
- On the right in `tx3`: "Counts exclude 1 monitoring gap" (cloud-off icon) when the interval overlaps a gap, or "Monitoring complete for this interval" when it does not. Zero events with healthy collection and missing events during an outage must read differently.

## Table

Columns: Time (108 px, monospace with milliseconds, `tx2`), Type (82 px), Process (118 px, monospace `name pid`), Action (72 px), Target (flexible, monospace, single line with ellipsis), Result (140–150 px).

- Newest first. Header row in `tx3` 12 px.
- Result: "Succeeded" in `tx2` (no green; green is reserved for running state); "Blocked" in `bad` with ban icon; failures in `bad` with an x icon and the specific reason ("Permission denied"); "Unknown" in `tx3`.
- Selected row: `#1B2130` background with a 2 px `acc` left bar.
- Lifecycle rows use `system` in `tx3` as the process.
- Cells are selectable and copyable; long targets show fully in the detail pane.

### Monitoring gap row

- Full-width row, placed at the gap's start time in the ordering.
- Background: 135° hatching in `hatch` at ~28% opacity (2 px stripe, 8 px period).
- Cloud-off icon, the interval in monospace `tx`, then the explanation in `tx2` 13 px: "Monitoring gap. The collector restarted and 212 events were dropped. Activity in this interval is unknown."
- Partial gaps name what is missing: "File events incomplete. 1,040 dropped." (see [agent-exited-degraded.md](agent-exited-degraded.md)).

## Detail pane

- Title by event type ("Process execution", "File write", "Blocked connection", …) 16 / 500, timestamp below in monospace `tx2` with its clock source ("guest clock"; host receipt time available on hover or expand).
- **Arguments** (process events): numbered argv list on `well`, index in `tx3`, values wrap anywhere. "Copy command" link in `acc` copies the full argv. Truncated arguments say "Truncated after 4 KB" in `warn`.
- Key/value rows (104 px key column in `tx3`, 13 px, 1 px `line` separators): Working directory, User (`agent uid 1000`), Process (`curl, pid 498`), Started by (parent chain, one per line with "from" in `tx3`), Attribution ("Agent descendant, from the guest collector"), Configuration ("Revision 7"), Not recorded ("Environment variables are not collected.").
- For file events: operation, guest path, mapped host path when available, result, bytes when captured.
- For policy events: action, matching rule, policy revision, decision reason.
- **Related** list: linked evidence (DNS lookup, blocked connection) with type icons; "Show in network view" link.
- Uncertainty is stated in the pane (attribution quality, truncation, loss) rather than hidden.

## Historical use

The same screen works for stopped instances and earlier runs. Each run shows the policy and configuration that applied at that time.
