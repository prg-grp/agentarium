# Create: review (step 5 of 5)

Last creation step. Flow rules are in [create-harness.md](create-harness.md#creation-flow-shared-by-all-five-steps).

## Layout

```
New instance
(✓) Harness  (✓) Agent  (✓) Shared folders  (✓) Network  (5) Review
──────────────────────────────────────────────────────────────────────
Harness         Name          billing-service                     Edit
                Agent         Claude Code
                Provisioning  Script from the Claude Code preset
──────────────────────────────────────────────────────────────────────
Agent           Runs       claude --permission-mode acceptEdits   Edit
                In         /home/ubuntu/workspace
──────────────────────────────────────────────────────────────────────
Shared folders  ~/code/billing-service  ✎ Read and write. Changes     Edit
                                          modify your files immediately.
                ~/code/shared-protos      Read only
──────────────────────────────────────────────────────────────────────
Network         Unlisted domains  Denied                              Edit
                Mac and local network  Blocked
                Allowed  api.anthropic.com, github.com, crates.io
                Denied   gist.github.com, pastebin.com
                Forwarded ports  18789 → random port
──────────────────────────────────────────────────────────────────────
Environment     Variables    RUST_LOG, DATABASE_URL                   Edit
──────────────────────────────────────────────────────────────────────
Resources       Processors [2 −+]  Memory (GB) [4 −+]  Disk (GB) [16 −+]
Defaults suit…  Your Mac has 12 processors and 36 GB of memory. 412 GB of disk is free.
                The disk can grow later, but not shrink.
──────────────────────────────────────────────────────────────────────
Start           Create only sets the instance up; start it later from its header.
──────────────────────────────────────────────────────────────────────
Cancel                       [< Back]  [Create]  [▶ Create and start]
```

## Elements

- Each section: title in the 220 px left column, key/value read-back (150 px key column in `tx3`, values in `tx` or monospace), "Edit" link in `acc` on the right that jumps to that step.
- Writable shares repeat the `warn` direct-write sentence in short form. Without folders: "Folders: None. The agent works on the instance's own disk."
- Harness shows the preset's name, and for provisioning "Script from the <preset> preset", "Template, not filled in yet" (custom agent, unchanged template), or "Custom script".
- Environment shows variable names only; values are not repeated.
- **Resources** are only editable here (the "advanced" section from the brief): steppers for processors, memory and disk, plus a `tx3` line with what the Mac has available and that the disk can grow later, but not shrink. Section description: "The virtual machine's processors, memory and disk. The defaults suit most agents. The instance takes memory and disk space from your Mac only as it uses them."
  - Defaults: 2 processors and 4 GB of memory, capped at half of the Mac. Claude Code is a Node process and builds inside the instance need memory; the VM takes memory from the Mac only as the guest uses it.
  - Limits: processors up to the Mac's count; memory up to the Mac's memory minus 2 GB; disk from 8 to 1024 GB, not capped by the Mac's free space, since it only takes what the guest writes.
  - Disk: 16 GB by default; the base system takes about 2. Coding agents mostly work in shared folders, so the disk holds the agent, caches and builds. Instance disks are sparse copy-on-write clones of the base image and take space on the Mac only as the guest writes.
- **Start:** a sentence explaining the two create buttons: "Create only sets the instance up; start it later from its header. You can change its settings before the first start."
- Footer: two create buttons. "Create" (neutral) creates the instance without starting it. "Create and start" (primary, play icon) also starts it right away, which runs its provisioning script first.

## States

- Blocking problems from earlier steps (missing folder, invalid command or guest path, duplicate variable, empty or unfilled script) are listed at the top of Review in `bad`, each with a "Go to <step>" link. Until they are fixed, the Start sentence says the instance can only be created without starting, and "Create and start" is disabled.
- A missing or duplicate name blocks creating altogether; both create buttons are disabled.
- If creating fails (for example, the base disk image is missing), the error shows in the footer in `bad`, and the draft stays as it is.
- After "Create and start", the instance's Terminal tab opens and shows provisioning stages (same stage list as [startup-failure.md](startup-failure.md), with stages filling in as they complete).
- Failures during provisioning keep all entered settings.
