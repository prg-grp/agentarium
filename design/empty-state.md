# Empty application

Shown when no instances exist. Frame: [app-shell.md](app-shell.md) sidebar only; there is no instance header or tab bar.

## Layout

```
┌──────────────────┬─────────────────────────────────────────────────────┐
│ ● ● ●            │                                                     │
│ Instances    [+] │          ▢                                          │
│ No instances yet │          Create an instance to run an AI agent      │
│                  │          with controlled file and network access.   │
│                  │                                                     │
│                  │          [+ Create instance]                        │
│ ⚙ Preferences    │                                                     │
└──────────────────┴─────────────────────────────────────────────────────┘
```

- The message block is centered in the main area, max width about 420 px, left-aligned text.
- Box icon (24 px, `tx3`) above the sentence.
- Sentence: 17 / 26, 400, `tx`.
- One primary button: "Create instance" with a plus icon.

## Copy

- Sidebar: "No instances yet"
- Main: "Create an instance to run an AI agent with controlled file and network access."
- Button: "Create instance"

## Behavior

- "Create instance" and the sidebar `+` open the creation flow at [create-agent.md](create-agent.md).
- No sample activity, onboarding tour, or dashboard with invented data.
