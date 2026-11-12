# Create: harness (step 1 of 5)

First step of instance creation. It names the instance and picks a preset: a known agent harness, or a custom agent. Implemented in `app/ui/create-instance.slint` and `app/src/create.rs`; the presets are in `app/src/preset.rs`. Implemented in `app/ui/create-instance.slint` and `app/src/create.rs`.

## Creation flow (shared by all five steps)

- Creation opens in the main area, not in a separate window or sheet. The sidebar keeps the instance list.
- As soon as the user enters a name, a draft item appears at the top of the sidebar: a pencil icon in `acc`, the name, and "Not created yet". The user can switch to other instances and come back; entered values persist until Cancel or Create. They are not saved across app restarts.
- Header: "New instance" (22 / 28, 500), then a `tx2` line: "An instance is a Linux virtual machine on this Mac with its own disk. The agent runs inside it and sees only the folders and domains you allow." The step indicator sits below it.
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
An instance is a Linux virtual machine on this Mac with its own disk. The agent runs inside it…
(1) Harness   (2) Agent   (3) Shared folders   (4) Network   (5) Review
───────────────────────────────────────────────────────────────────────
Name and harness    Name     [billing-service        ]
The harness is the  Harness  (•) Claude Code    Anthropic's coding agent
agent that runs…             ( ) Codex          OpenAI's coding agent
                             ( ) Gemini CLI     Google's coding agent
                             ( ) OpenCode       Open-source agent; add your model provider's domain
                             ( ) Custom agent   Bring your own provisioning script and command
───────────────────────────────────────────────────────────────────────
Provisioning script Show script
Defined by the
Claude Code preset…
───────────────────────────────────────────────────────────────────────
Cancel                                              [Continue to agent]
```

## Presets

A preset is compiled into the app and defines three things:
- the launch command, without arguments;
- the provisioning script (`vms/provision/<preset>.sh`);
- the initial network rules: the domains the agent needs, allowed.

A preset only exists while the instance is drafted. Creating the instance copies these values into its first revision; the chosen preset isn't stored, and later app versions don't change existing instances.

The command and script of a preset can't be changed. The only way to use others is Custom agent. The network rules are only the initial selection: the user can change or remove them on the Network step, like any rule.

## Elements

- **Name:** text field, max width 320 px. Required, and unique among instances (case-insensitive). Used in the sidebar and header.
- **Harness:** radio list of the presets, in the order of `Preset::ALL`, each with a one-line note in `tx3`. Claude Code is preselected.
- **Provisioning script:**
  - For a preset: a "Show script" / "Hide script" toggle (ghost, compact) reveals the script read-only in the monospace editor. Description: "Defined by the <preset> preset. Runs once as root with internet access before the first start. Choose Custom agent to write your own."
  - For a custom agent: a monospace multi-line editor, about 320 px high, without wrapping, prefilled with the template `vms/provision/custom.sh`. "Reset to template" (ghost, compact) appears once the script differs from it. Description: "Runs once as root with internet access before the first start. Must be safe to run again."

## Behavior

- Switching preset sets the launch command and the provisioning script to the new preset's.
- Switching away from Custom agent keeps its command and script in the draft, so switching back restores them.
- Switching preset also swaps the network rules. The old preset's rules that the user left unchanged are removed, and the new preset's rules are added unless the same target is already in the same list. Rules the user added or changed stay.
- A custom agent whose script is still the unchanged template can be created but not started. Review lists "Fill in the provisioning script so it installs your agent."
- An empty custom script is a blocking problem on Review.
- A duplicate name shows "Another instance already uses this name." below the field, which is outlined in `bad`.

## Copy

- Section titles: "Name and harness", "Provisioning script"
- Primary button: "Continue to agent"
