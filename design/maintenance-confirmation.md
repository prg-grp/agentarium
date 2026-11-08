# Maintenance confirmation

The dialog shown before entering maintenance mode, from "Restart in maintenance" (running instance) or "Boot in maintenance" (stopped instance). The session itself: [maintenance-session.md](maintenance-session.md).

## Layout

```
┌──────────────────────────────────────────────────────────────┐
│ Restart api-refactor in maintenance?                         │
│ For installing software or changing the instance. It applies │
│ to this one boot only.                                       │
│ ──────────────────────────────────────────────────────────── │
│ ⏸  Claude Code won't start.                                  │
│ ──────────────────────────────────────────────────────────── │
│ ⌘  The terminal opens a root shell inside the instance.      │
│ ──────────────────────────────────────────────────────────── │
│ 🌐 Internet rules are lifted until the next restart. Your Mac │
│    and local network stay blocked.                           │
│ ──────────────────────────────────────────────────────────── │
│ 📁 These folders can still change files on your Mac:          │
│    ~/code/api  read and write                                │
│ ──────────────────────────────────────────────────────────── │
│ ♥  Monitoring keeps running, but a root shell can change or  │
│    stop it.                                                  │
│                         [Cancel] [⚒ Restart in maintenance]  │
└──────────────────────────────────────────────────────────────┘
```

## Elements

- Modal dialog: `panel`, 1 px `line2` border, 12 px radius, 24 px padding, about 520 px wide.
- Title 17 / 500. For a stopped instance: "Boot docs-site in maintenance?"
- Intro in `tx2`.
- Effect list: one item per row, 20 px icon column, 1 px `line` separators. Network and folder items use `warn` icons; others `tx2`.
- The folder item lists every enabled read-and-write share by host path (monospace) with "read and write" in `tx3`. If there are none, the item reads "No shared folders can change files on your Mac."
- Actions: "Cancel" (ghost) and "Restart in maintenance" / "Boot in maintenance" (mode-confirm: `warn` fill, text `#2A1E06`, tool icon). Escape cancels; Return does not confirm by default.

## Rules

- State concrete effects, not just the mode name.
- The network item must say what stays blocked, not only what opens. When the instance allows its Mac and local network, the second sentence reads "Your Mac and local network are reachable too."
- For a running instance, add "The running agent session will end." as the first item.
