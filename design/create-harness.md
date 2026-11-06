# Create: harness (step 1 of 5)

First step of instance creation. It names the instance, picks the agent harness, and holds the provisioning script that installs the harness. Implemented in `app/ui/create-instance.slint` and `app/src/create.rs`.

## Creation flow (shared by all five steps)

- Creation opens in the main area, not in a separate window or sheet. The sidebar keeps the instance list.
- As soon as the user enters a name, a draft item appears at the top of the sidebar: a pencil icon in `acc`, the name, and "Not created yet". The user can switch to other instances and come back; entered values persist until Cancel or Create. They are not saved across app restarts.
- Header: "New instance" (22 / 28, 500) with a step indicator below it.
- Step indicator: Harness, Agent, Shared folders, Network, Review. Each step has a 22 px circle:
  - The current step is filled `acc`, with its number in dark text.
  - Visited steps show a check in `ok` with an `ok` border.
  - Future steps show the number in `tx3`.
  - The steps are a real sequence, so numbering is appropriate. Visited steps can be clicked to jump back.
- Footer bar (1 px `line` top border): "Cancel" (ghost) on the left; "Back" (ghost) and the primary "Continue to …" button on the right. The primary label names the next step.
- Sections use a two-column layout: a 220 px left column holds the section title (15 / 500) and a short description in `tx3` 13 px; a right column (max 640 px) holds the fields. Sections are separated by 1 px `line` rules.
- Errors never block navigation. Problems that block starting or creating are listed again on Review.

Related steps: [create-agent.md](create-agent.md), [create-shared-folders.md](create-shared-folders.md), [create-network.md](create-network.md), [create-review.md](create-review.md).

## Layout

```
New instance
(1) Harness   (2) Agent   (3) Shared folders   (4) Network   (5) Review
───────────────────────────────────────────────────────────────────────
Name and harness    Name     [billing-service        ]
The harness is the  Harness  (•) Claude Code
agent that runs…             ( ) Codex          Not available yet
                             ( ) Custom agent   Bring your own provisioning script
───────────────────────────────────────────────────────────────────────
Provisioning script ┌───────────────────────────────────────────────┐
Runs once as root   │#!/bin/bash                                    │
with internet…      │# Default provisioning script for Claude Code… │
                    └───────────────────────────────────────────────┘
                    Reset to default
───────────────────────────────────────────────────────────────────────
Cancel                                              [Continue to agent]
```

## Elements

- **Name:** text field, max width 320 px. Required, and unique among instances (case-insensitive). Used in the sidebar and header.
- **Harness:** radio list.
  - Claude Code is preselected.
  - Codex is visible but disabled, with "Not available yet" in `tx3`, so later support is discoverable.
  - Custom agent is selectable, with "Bring your own provisioning script" in `tx3`.
- **Provisioning script:** a monospace multi-line editor, about 320 px high, without wrapping.
  - It is prefilled with the harness's built-in script: `vms/provision/claude-code.sh` for Claude Code, or the template `vms/provision/custom.sh` for a custom agent. Both are compiled into the app.
  - The user can edit the script for any harness.
  - "Reset to default" (ghost, compact) appears once the script differs from the built-in one.
  - Description: "Runs once as root with internet access before the first start. Must be safe to run again."

## Behavior

- Switching harness replaces the launch command and the provisioning script with the new harness's defaults, but only where the user hasn't changed them. A custom agent has no default command.
- A custom agent whose script is still the unchanged template can be created but not started. Review lists "Fill in the provisioning script so it installs your agent."
- An empty script is a blocking problem on Review.
- A duplicate name shows "Another instance already uses this name." below the field, which is outlined in `bad`.

## Copy

- Section titles: "Name and harness", "Provisioning script"
- Primary button: "Continue to agent"
