# Startup failure

An instance whose boot stopped before the agent launched. Frame: [app-shell.md](app-shell.md). The same stage list is used while provisioning after "Create and start" ([create-review.md](create-review.md)) and during every normal start.

## Layout

```
infra-audit                                                     [Restart in maintenance] [Stop] │ Delete
Claude Code
───────────────────────────────────────────────────────────────────────────────────────────────────────
Terminal   Logs   Status   Activity   Settings
───────────────────────────────────────────────────────────────────────────────────────────────────────
The agent wasn't started                           │ Boot log      Serial console, not the agent session
Monitoring didn't come up, so nothing ran without  │ [    0.000000] Linux version 6.12.4-toolbox (arm64)
being observed. The virtual machine is still       │ [    1.214882] virtio-fs: mounted work-infra at /work/infra
running.                                           │ [    2.003117] toolbox-supervisor: control channel up
✓ Boot virtual machine                     4.1 s   │ …
✓ Mount shared folders                     0.3 s   │ [    2.311902] toolbox-collector: probe … invalid argument
✓ Apply network rules                      0.8 s   │ [    2.312001] toolbox-collector: exited with status 1
✗ Start monitoring                        20.0 s   │ [   22.004873] toolbox-supervisor: collector handshake timed out
  The activity collector didn't respond within     │ [   22.005010] toolbox-supervisor: agent launch withheld
  20 seconds.                                      │
  This often follows a guest update made in        │
  maintenance mode. Retry, or open a maintenance   │
  session to inspect the collector.                │
◌ Start Claude Code                       Skipped  │
[↻ Retry start]  Copy diagnostics                  │ Sample data
```

- Two columns: stages (max 520 px, 28 px padding) and the boot log (flexible, `well` background, 1 px `line` left border).

## Header and Status tab

- The Status tab's readings are independent: Virtual machine "Running" (`ok`), Agent "Not started" (`tx2`, pause icon in `tx3`), Monitoring "Failed to start" (`bad`).
- If the VM process is gone, Virtual machine reads "Failed" in `bad` and the actions become "Start" / "Boot in maintenance".
- Sidebar item: alert-triangle in `bad`, "Startup failed".

## Stage list

- Headline 17 / 500 and a one-paragraph explanation in `tx2` (max 440 px).
- Stages in order: Boot virtual machine, Mount shared folders, Apply network rules, Start monitoring, Start Claude Code. Each row: status icon, name, duration in monospace `tx3` right-aligned.
  - Done: circle-check `ok`.
  - Running (during normal startup): spinner-free indicator, e.g. a circle-dashed icon in `acc` with "In progress"; no animated effects beyond the platform's progress indicator.
  - Failed: circle-x `bad`, then a `bad` 13 px sentence with what happened and a `tx2` 13 px sentence with what to do.
  - Not reached: circle-dashed in `tx3`, "Skipped".
- The agent never launches before policy enforcement and monitoring succeed. The copy must say so ("nothing ran without being observed").
- Actions: "Retry start" (primary, refresh icon) and "Copy diagnostics" (ghost). Copied diagnostics never include secrets.

## Boot log

- Header: "Boot log" (`tx2`, 500) and "Serial console, not the agent session" (`tx3`).
- Monospace 12 / 20 in `tx2`, kernel timestamps in `tx3`, error lines in `bad`. Text is untrusted input: strip or neutralize terminal escape sequences.
- Technical terms are acceptable here; this is the diagnostics surface.

## Other failure copy (same pattern)

- Mount: "~/code/legacy-billing no longer exists on your Mac. Choose another folder in Settings, then retry."
- Network: "Couldn't apply network rules. The agent wasn't started. Retry, or check the rules in Settings."
- Launch: "The command `claud` wasn't found inside the instance. Check the command in Settings."
- Disk: "The instance disk is full. Free space in maintenance mode or increase the disk size."
