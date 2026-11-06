# Status tab

The instance's state at a glance: the three readings and, in agent mode, the guest's live resource use. Frame: [app-shell.md](app-shell.md). Tokens and the status vocabulary come from [DESIGN.md](../DESIGN.md).

The guest supervisor reports every 2 s, and at once when the agent starts or exits. The app keeps only the latest report; nothing is stored, so the tab has no history and no chart over time.

## Layout

```
api-refactor                                        [Restart in maintenance] [Stop] │ Delete
Claude Code
──────────────────────────────────────────────────────────────────────────────────────────────
Terminal   Logs   Status   Activity   Settings
──────────────────────────────────────────────────────────────────────────────────────────────
▣ Virtual machine     ✦ Agent        ♡ Monitoring
  Running               Running        Not installed
──────────────────────────────────────────────────────────────────────────────────────────────
Resource use   Updated every 2 s

CPU                                          Agent 38% · VM 52% of 4 vCPUs
[██████████████ ▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░░░░░░]

Memory                                 Agent 1.4 GB · VM 2.1 GB of 3.8 GB
[██████████████ ▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░░░░]

■ Agent   ■ Rest of the VM   ■ Free

Agent
Restarts this boot                                                       1
──────────────────────────────────────────────────────────────────────────
Last exit                                                         Status 0
──────────────────────────────────────────────────────────────────────────
Processes ended for lack of memory                                       0
```

- Content: 24 px top padding, 28 px side padding, 28 px between the readings, the divider and the resources. The resources column is at most 720 px wide.

## Readings

- Three readings in a row, 48 px apart: icon (18 px), label in `tx3` 12 / 16, value 14 / 20 in the state color. Values are in [DESIGN.md](../DESIGN.md#status-vocabulary).
- Agent icon: sparkles while running or starting, alert-triangle in `warn` when reports are overdue ("Unknown"), square-x after an exit in agent mode, otherwise pause.
- The supervisor starts the agent again 2 s after it exits, so "Exited, status *n*" shows only briefly; "Restarts this boot" and "Last exit" keep the record.
- Monitoring reads "Not installed" until the guest has an activity collector.

## Meters

- One stacked meter each for CPU and memory: a 10 px track in `raise` with 4 px rounded ends. The agent's part (`tx2`) comes first, then the rest of the VM (`tx3`), separated by a 2 px gap; the remainder of the track is free. The two greys are an ordinal pair: the agent is part of the VM.
- CPU is a share of all vCPUs over the last report interval. Memory is a share of the memory the guest sees, without cache the kernel can reclaim.
- The value text on the right of the label (`tx2`, 13 px) names both numbers, so the meter is never read from color alone. Until two reports are in, CPU reads "Measuring…".
- A legend under the meters: Agent, Rest of the VM, Free.
- Memory warning: when the guest's memory is at least 90% used, or the agent's cgroup has had a process ended for lack of memory, the agent segment and its legend key turn `warn`, and a line with alert-triangle in `warn` says "The VM's memory is almost full." or "The agent ran out of memory. Give the instance more memory in its settings."
- If the agent's own use can't be measured (its cgroup couldn't be set up), the meters show only the VM, the legend reads "VM" and "Free", and a `tx3` line says "The agent's own use can't be measured in this instance; the meters show the whole VM." The out-of-memory row is hidden.

## Overdue reports

- After 10 s without a report: the status line next to "Resource use" turns `warn` with alert-triangle, "No status received for over 10 s". The meters dim to 40% and their texts to `tx3`; they keep the last values instead of pretending to be live.
- Agent and Monitoring read "Unknown" in `warn`. The next report restores everything.

## Other states

The readings stay; the resources are replaced by one sentence in `tx3`, 13 px:

| State | Sentence |
|---|---|
| Stopped or startup failed | "Start the instance to see its resource use." |
| Agent mode, before the first report | "Waiting for the first status report." |
| Maintenance | "Resource use is reported in agent mode only." |
| Provisioning | "Resource use is reported once provisioning has finished." |
| Stopping | "The virtual machine is stopping." |

No overdue warning outside agent mode.
