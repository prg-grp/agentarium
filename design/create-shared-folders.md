# Create: shared folders (step 2 of 4)

Second creation step. Flow rules (draft in sidebar, step indicator, footer) are in [create-agent.md](create-agent.md#creation-flow-shared-by-all-four-steps). The same table is reused in Settings ([settings-pending.md](settings-pending.md)).

## Layout

```
Shared folders                                                     [Add folder]
The agent sees these folders live. Nothing else on your Mac is shared.

On your Mac               Inside the instance        Access
──────────────────────────────────────────────────────────────────────────────
~/code/billing-service    [/work/billing-service]    [Read only|Read and write]  ×
⚠ Changes inside this instance modify these host files immediately.
──────────────────────────────────────────────────────────────────────────────
~/code/shared-protos      [/work/shared-protos]      [Read only|Read and write]  ×
──────────────────────────────────────────────────────────────────────────────
~/code/legacy-billing     [/work/legacy-billing]     [Read only|Read and write]  ×
(struck through)
⊘ This folder no longer exists on your Mac. Choose another folder or remove it. Choose folder
──────────────────────────────────────────────────────────────────────────────
Cancel                                          [< Back]  [Continue to network]
```

## Elements

- Title "Shared folders" (16 / 500) and the description in `tx2`.
- "Add folder" (neutral, folder-plus icon) opens the macOS folder picker.
- Table columns: On your Mac (monospace, wraps anywhere), Inside the instance (editable monospace field on `well`), Access (segmented control), remove (icon-only ghost, labeled "Remove folder"). Column header row in `tx3` 12 px.
- Guest path default: `/work/<folder name>`. Keep host-to-guest mapping stable for activity display.
- Access segmented control labels: "Read only" and "Read and write". New folders default to Read only.

## States

- **Read and write:** a line under the row in `warn` 13 px with an alert icon: "Changes inside this instance modify these host files immediately."
- **Missing or inaccessible folder:** host path in `tx2` with a `tx3` strike-through; guest field in `tx2`; a line under the row in `bad` with a folder-off icon: "This folder no longer exists on your Mac. Choose another folder or remove it." followed by a "Choose folder" link in `acc`. For access failures: "The app can't read this folder. Allow access in System Settings, or choose another folder."
- **Duplicate guest path:** error beside the second guest field: "Another folder already uses this path inside the instance."
- Continue stays enabled; blocking errors are listed again on Review and prevent start, not navigation.

## Not offered

- No copy/diff or "protected" mode. Do not show it as an option, even disabled.
- No implicit sharing of the home folder, credential stores, or unrelated projects.
