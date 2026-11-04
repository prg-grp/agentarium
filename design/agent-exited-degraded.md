# Agent exited, monitoring degraded

A running VM whose agent process exited, while monitoring is partly working. It shows two independent states at once: "VM running / agent exited" and "monitoring degraded". Frame: [app-shell.md](app-shell.md). Terminal and rail details: [terminal-running.md](terminal-running.md).

## Layout

```
api-refactor  │ VM Running │ Agent Exited, status 1 │ Monitoring Degraded │ [Restart in maintenance] [Stop] │ Delete
Claude Code
──────────────────────────────────────────────────────────────────────────────────────────────────────
Terminal   Activity   Settings                                                        [Live activity]
──────────────────────────────────────────────────────────────────────────────────────────────────────
⚠ Monitoring is degraded since 14:19:40. 1,040 file events were dropped because the buffer filled.
  Process and network events are still recorded, and network rules are still enforced.  Show affected interval
──────────────────────────────────────────────────────────────────────────────────────────────────────
(dimmed retained output)                                     │ Live activity                     Pause
● Bash(cargo test -p api)                                    │ 14:22:07 ⏻ Agent exited
  └ test result: ok. 214 passed; 0 failed                    │            Status 1
● All upload tests pass with the new limiter.                │ 14:21:12 ⇌ 160.79.104.10:443
Error: request to api.anthropic.com timed out after 600s     │            Connection succeeded, …
claude exited                                                │ ▨ 14:19:40 to now
─────────────────────────────────────────────────────────────│   File events incomplete. 1,040 dropped.
☒ Claude Code exited with status 1 at 14:22:07               │ 14:19:31 ⌘ cargo test -p api
  Output above is kept until the next start.                 │            Executed by claude
                        [Reboot instance] [▶ Restart agent]  │ Sample data
```

## Header

- Agent: square-x icon in `tx2`, value "Exited, status 1" in `tx`. An exit is not automatically an error; status 0 reads "Exited, status 0".
- Monitoring: heartbeat in `warn`, "Degraded".
- Sidebar summary: "Agent exited". If monitoring is also degraded, the sidebar icon stays the running VM icon; details live in the header.

## Degraded monitoring notice

- Full-width bar under the tab bar: `warn` at 8% fill, 1 px `line` bottom border, alert-triangle icon in `warn`.
- First sentence in `warn`: when it started. Then in `tx2`: what is missing and why, and what still works. Always state whether network enforcement is affected (it runs on the host, so normally it is not).
- Action link "Show affected interval" in `acc` opens the Activity timeline filtered to the gap.
- The notice is visible on every tab while the condition lasts. It does not compete with the terminal: one line, no fill on the terminal itself.
- Variants:
  - Collector stopped: "Monitoring stopped at 14:19:40. No file, process, or network events are being recorded inside the instance. Network rules are still enforced."
  - Transport interrupted: "Connection to the instance's monitoring was interrupted at 14:19:40. Events may arrive late or be missing."
  - Storage failure: "Activity can't be saved: the disk is full. Events since 14:19:40 are not stored."

## Exited agent in the terminal

- Retained output stays visible, dimmed (about 72% opacity is acceptable for the mock; in the product use `tx2`-level contrast, not opacity on colored text).
- Exit bar pinned to the bottom of the terminal: `panel` fill, 1 px `line2` top border, square-x icon, "Claude Code exited with status 1 at 14:22:07" and "Output above is kept until the next start." in `tx3`.
- Actions: "Restart agent" (neutral, play icon) restarts only the agent process in the same VM and run. "Reboot instance" (ghost) restarts the VM. The two must be clearly distinct.

## Live activity rail

- The partial gap row is hatched (see [activity-timeline.md](activity-timeline.md#monitoring-gap-row)) and placed at the gap's start time: "14:19:40 to now / File events incomplete. 1,040 dropped."
- Events of still-working categories continue to appear above the gap row.
