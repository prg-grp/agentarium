# Empty application

Shown when no instances exist. Frame: [app-shell.md](app-shell.md) sidebar only; there is no instance header or tab bar.

## Layout

```
┌──────────────────┬─────────────────────────────────────────────────────┐
│ ● ● ●            │                                                     │
│ Instances    [+] │          ▢                                          │
│ No instances yet │          Create an instance to run an AI agent      │
│                  │          in its own virtual machine.                │
│                  │                                                     │
│                  │          ▣ Each instance is a Linux virtual         │
│                  │            machine on this Mac, with its own disk.  │
│                  │          ▤ The agent sees only the folders you      │
│                  │            share with it.                           │
│                  │          ⇄ It reaches only the domains you allow.   │
│                  │                                                     │
│                  │          [+ Create instance]                        │
│ ⚙ Preferences    │                                                     │
└──────────────────┴─────────────────────────────────────────────────────┘
```

- The message block is centered in the main area, max width about 420 px, left-aligned text.
- Box icon (24 px, `tx3`) above the sentence.
- Sentence: 17 / 26, 400, `tx`.
- Below it (16 px gap), three lines 8 px apart: a 16 px `tx3` icon (server, folder, crossing arrows) and the text in `tx2` 13 px. They explain the model in plain words; they are not a tour.
- One primary button: "Create instance" with a plus icon.

## Copy

- Sidebar: "No instances yet"
- Main: "Create an instance to run an AI agent in its own virtual machine."
- Lines: "Each instance is a Linux virtual machine on this Mac, with its own disk." / "The agent sees only the folders you share with it." / "It reaches only the domains you allow."
- Button: "Create instance"

## Behavior

- "Create instance" and the sidebar `+` open the creation flow at [create-agent.md](create-agent.md).
- No sample activity, onboarding tour, or dashboard with invented data.
