# Create: review (step 4 of 4)

Last creation step. Flow rules are in [create-agent.md](create-agent.md#creation-flow-shared-by-all-four-steps).

## Layout

```
New instance
(✓) Agent  (✓) Shared folders  (✓) Network  (4) Review
──────────────────────────────────────────────────────────────────────
Agent           Name       billing-service                        Edit
                Agent      Claude Code
                Runs       claude --permission-mode acceptEdits
                In         /work/billing-service
                Sign-in    API key from Keychain
──────────────────────────────────────────────────────────────────────
Shared folders  ~/code/billing-service  ✎ Read and write. Changes     Edit
                                          modify your files immediately.
                ~/code/shared-protos      Read only
──────────────────────────────────────────────────────────────────────
Network         Unlisted domains  Denied                              Edit
                Allowed  api.anthropic.com, github.com, crates.io
                Denied   gist.github.com, pastebin.com
──────────────────────────────────────────────────────────────────────
Environment     Variables    RUST_LOG, DATABASE_URL                   Edit
                In Keychain  STRIPE_SECRET_KEY
──────────────────────────────────────────────────────────────────────
Resources       Processors [4 −+]  Memory [8 GB −+]  Disk [64 GB −+]
Defaults suit…  Your Mac has 12 processors and 36 GB of memory. 412 GB of disk is free.
──────────────────────────────────────────────────────────────────────
Cancel                  [< Back] [Create without starting] [▶ Create and start]
```

## Elements

- Each section: title in the 220 px left column, key/value read-back (150 px key column in `tx3`, values in `tx` or monospace), "Edit" link in `acc` on the right that jumps to that step.
- Writable shares repeat the `warn` direct-write sentence in short form.
- Environment shows variable names only; values are not repeated. Keychain items are listed by name.
- **Resources** are only editable here (the "advanced" section from the brief): steppers for processors, memory, and disk with usable defaults, plus a `tx3` line with what the Mac has available.
- Footer: "Create without starting" (neutral) and "Create and start" (primary, play icon).

## States

- Blocking problems from earlier steps (missing folder, invalid command) are listed at the top of Review in `bad` with links to the step. "Create and start" is replaced by "Create without starting" as the only create action until they are fixed.
- After "Create and start", the instance's Terminal tab opens and shows provisioning stages (same stage list as [startup-failure.md](startup-failure.md), with stages filling in as they complete).
- Failures during provisioning keep all entered settings.
