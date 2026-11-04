# Create: agent (step 1 of 4)

First step of instance creation. The same sections are reused in Settings ("Agent and launch", "Environment"), see [settings-pending.md](settings-pending.md).

## Creation flow (shared by all four steps)

- Creation opens in the main area, not a separate window or sheet. The sidebar keeps the instance list.
- As soon as the user enters a name, a draft item appears at the top of the sidebar: pencil icon in `acc`, the name, "Not created yet". The user can switch to other instances and come back; entered values persist.
- Header: "New instance" (22 / 28, 500) and a step indicator below it.
- Step indicator: Agent, Shared folders, Network, Review. Each step has a 22 px circle: current step filled `acc` with its number in dark text; completed steps show a check in `ok` with an `ok` border; future steps show the number in `tx3`. Steps are a real sequence, so numbering is appropriate. Completed steps can be clicked to jump back.
- Footer bar (1 px `line` top border): "Cancel" (ghost) on the left; "Back" (ghost) and the primary "Continue to …" button on the right. The primary label names the next step.
- Sections use a two-column layout: a 220 px left column with the section title (15 / 500) and a short description in `tx3` 13 px, and a right column (max 640 px) with the fields. Sections are separated by 1 px `line` rules.

Related steps: [create-shared-folders.md](create-shared-folders.md), [create-network.md](create-network.md), [create-review.md](create-review.md).

## Layout

```
New instance
(1) Agent   (2) Shared folders   (3) Network   (4) Review
───────────────────────────────────────────────────────────
Name and agent      Name   [billing-service        ]
                    Agent  (•) Claude Code
                           ( ) Codex          Not available yet
                           ( ) Custom agent   Not available yet
───────────────────────────────────────────────────────────
Launch              Command [claude]  Arguments [--permission-mode][acceptEdits] Add argument
What starts when…   Working directory inside the instance [/work/billing-service]
───────────────────────────────────────────────────────────
Sign-in             ( ) Sign in inside the terminal on first start
Kept in Keychain…   (•) Use an Anthropic API key   [sk-ant-••••3f9a] [Replace key]
───────────────────────────────────────────────────────────
Environment         [RUST_LOG        ] [info                     ] ×
variables           [DATABASE_URL    ] [postgres://db.test/billing] ×
Visible to agent…   [STRIPE_SECRET_KEY] [sk_test_51H••••          ] ×
                    ⚿ This looks like a secret. Values here are stored in plain text. Move to Keychain
                    + Add variable
───────────────────────────────────────────────────────────
Cancel                                       [Continue to shared folders]
```

## Elements

- **Name:** text field, max width 320 px. Required. Used in the sidebar and header.
- **Agent:** radio list. Claude Code is the only selectable option. Codex and Custom agent are visible but disabled with "Not available yet" in `tx3`, so later support is discoverable.
- **Launch:**
  - Command field (200 px, monospace) prefilled with the agent default (`claude`).
  - Arguments field showing each argument as a token (monospace on `raise`, 4 px radius) plus an "Add argument" affordance. Arguments are an argv list, not a single string.
  - Working directory inside the instance (monospace, max 360 px). Default: the guest path of the first shared folder.
  - Description: "What starts when the instance boots. Runs directly, not through a shell." If shell execution is supported later, it must be an explicit option.
- **Sign-in:** radio list. "Sign in inside the terminal on first start" or "Use an Anthropic API key". The key field is masked (show only the last four characters) with a "Replace key" ghost button. Description: "Kept in your macOS Keychain, never in activity or settings history."
- **Environment variables:** two-column rows (name 200 px, value flexible, both monospace) with a remove icon. "Add variable" ghost button below. Description: "Visible to the agent and recorded with each run. Put secrets under sign-in instead."

## Validation and states

- If a variable name or value looks like a secret (names containing KEY, SECRET, TOKEN, PASSWORD, or values with known key prefixes), outline the name field in `bad` and show a `warn` line below the row: key icon, "This looks like a secret. Values here are stored in plain text." and a "Move to Keychain" link in `acc`. Moving it removes the plain value and stores a Keychain reference.
- Invalid or empty command: error beside the command field, e.g. "Enter the program to start, like claude."
- Duplicate variable names: error beside the second occurrence.
- Errors never clear other entered values.

## Copy

- Section titles: "Name and agent", "Launch", "Sign-in", "Environment variables"
- Primary button: "Continue to shared folders"
