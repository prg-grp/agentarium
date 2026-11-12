# Create: agent (step 2 of 5)

Second creation step. The flow rules (draft in the sidebar, step indicator, footer, section layout) are in [create-harness.md](create-harness.md#creation-flow-shared-by-all-five-steps). Settings reuses the same sections ("Agent and launch", "Environment"); see [settings-pending.md](settings-pending.md).

## Layout

```
New instance
(✓) Harness   (2) Agent   (3) Shared folders   (4) Network   (5) Review
───────────────────────────────────────────────────────────────────────
Launch              Command [claude]
What starts when…   Arguments [--permission-mode ×][acceptEdits ×] [Add argument]
                    Working directory inside the instance [/home/ubuntu/workspace]
───────────────────────────────────────────────────────────────────────
Environment         [RUST_LOG        ] [info                     ] ×
variables           [DATABASE_URL    ] [postgres://db.test/billing] ×
Visible to agent…   + Add variable
───────────────────────────────────────────────────────────────────────
Cancel                              [< Back]  [Continue to shared folders]
```

## Elements

There is no sign-in setting: the agent signs in inside the terminal on its first start.


- **Launch:**
  - Command field (200 px, monospace), set by the preset (`claude`, `codex`, `gemini`, `opencode`, `openclaw`) and read-only, with "Set by the <preset> preset." in `tx2` beside it. It is editable, and starts empty, only for a custom agent.
  - Arguments shown as tokens (monospace on `raise`, 4 px radius, each with a remove icon), followed by an "Add argument" field. They start with the preset's arguments, if any (OpenClaw: `gateway run --port 18789`). Return adds the typed argument. Arguments form an argv list, not a single string; each is passed as it is.
  - Working directory inside the instance (monospace, max 360 px). Default: `/home/ubuntu/workspace`, the agent user's `~/workspace`, where shared folders are placed by default. A `tx3` hint below the field: "~/workspace is on the instance's own disk. Shared folders go below it by default."
  - Description: "What starts when the instance boots. Runs directly, not through a shell." If shell execution is supported later, it must be an explicit option.
- **Environment variables:** two-column rows (name 200 px, value flexible, both monospace), each with a remove icon. An "Add variable" ghost button sits below. Description: "Visible to the agent and recorded with each run."
  - Values are stored as they are, in plain text, and passed to the instance as they are. A secret mode (Keychain-backed values) comes later; until then there is no secret detection or warning.
  - Rows left completely empty are dropped on create.

## Validation and states

- Empty command: an error beside the command field: "Enter the program to start, like claude."
- Working directory that isn't an absolute path: an error below the field.
- Variable name with a value but no name, a name containing `=` or spaces, or a duplicate name: the name field is outlined in `bad` with an error below the row. Duplicates are reported on the second occurrence.
- Errors never clear other entered values.

## Copy

- Section titles: "Launch", "Environment variables"
- Primary button: "Continue to shared folders"
