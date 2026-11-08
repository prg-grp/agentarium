# Create: network (step 4 of 5)

Fourth creation step. Flow rules are in [create-harness.md](create-harness.md#creation-flow-shared-by-all-five-steps). The same editor is reused in Settings ([settings-pending.md](settings-pending.md)). Policy semantics come from [ARCHITECTURE.md](../ARCHITECTURE.md#network-policy).

## Layout

```
Destinations not listed below                              │ How rules match
[Deny|Allow]  The agent can reach only the domains you allow.│ github.com matches that exact
                                                            │ name. Turn on subdomains to
Your Mac and local network                                  │
[Block|Allow]  The agent can't reach your Mac or devices on…│
                                                            │
✓ Allowed                       ⊘ Denied                    │ include api.github.com…
api.anthropic.com  Exact name ×  gist.github.com Exact name × │
github.com   Includes subdomains × pastebin.com Incl. subdom. ×│ When a domain is both allowed
crates.io    Includes subdomains × 10.0.0.0/8  Address range ×│ and denied, it's denied.
203.0.113.7/32  Single address × [Add a domain or address…]│
gist.github.com    Exact name                               │
 ⇄ Also denied below. Deny rules win, so this domain        │ Domain rules use DNS results…
   stays blocked.                                           │
[https://pypi.org/simple]  (red outline)                    │ ⌂ Your Mac and local network
 Enter a domain like pypi.org, an address like 203.0.113.7, │   stay unreachable, whatever
 or a range like 10.0.0.0/8.                                │
                                                            │   the rules say.
Cancel                                        [< Back]  [Continue to review]
```

- Main column plus a 280 px help column separated by a 1 px `line` border, 24 px padding.

## Elements

- **Initial rules:** the lists start with the preset's rules (see [create-harness.md](create-harness.md#presets)), such as `api.anthropic.com` for Claude Code. They are ordinary rules: the user can change their scope or remove them. A custom agent starts with no rules.
- **Default action:** heading "Destinations not listed below" with a segmented control "Deny | Allow" (Deny is the default) and one explanatory sentence that changes with the choice:
  - Deny: "The agent can reach only the destinations you allow."
  - Allow: "The agent can reach any destination except the ones you deny."
- **Mac and local network:** heading "Your Mac and local network" with a segmented control "Block | Allow" (Block is the default) and one sentence:
  - Block: "The agent can't reach your Mac or devices on your network, whatever the rules say."
  - Allow: "Local addresses follow the rules. Your Mac is host.internal."
  - When allowed, a `warn` message with the alert-triangle icon below it. With default Deny: "The agent can reach the local addresses you allow. Allow 192.168.127.1 for services on your Mac." With default Allow: "The agent can reach services on your Mac and other devices on your network."
  - The setting applies to every boot, maintenance included. See [ARCHITECTURE.md](../ARCHITECTURE.md#decided-policy).
- **Allowed** and **Denied** lists side by side, each headed by an icon (`ok` circle-check, `bad` ban) and the word. Each rule row: the target (monospace), scope text in `tx2` 12 px, and a remove icon. An input at the bottom of each list ("Add a domain or address to allow" / "… to deny"); Return adds the entry.
- **Rule targets:** a rule is either a domain or an address range.
  - Domain (`pypi.org`): the scope reads "Exact name" or "Includes subdomains" and toggles on click. Stored lowercase, without a trailing dot.
  - IP address (`203.0.113.7`, IPv4 or IPv6): stored as a single-address range (`/32` or `/128`). The scope reads "Single address".
  - CIDR range (`10.0.0.0/8`): the scope reads "Address range". Host bits are cleared, so `10.1.2.3/8` is stored as `10.0.0.0/8`.
- **Help column** ("How rules match"):
  - "github.com matches that exact name. Turn on subdomains to include api.github.com and others."
  - "Addresses and ranges, like 203.0.113.7 or 10.0.0.0/8, match the destination directly, without DNS."
  - "When a destination is both allowed and denied, it's denied."
  - "Domain rules use DNS results to control destination addresses. A recorded address may be associated with more than one domain."
  - With a home-off icon: "Your Mac and local network stay unreachable, whatever the rules say." When the Mac and local network are allowed, the icon becomes alert-triangle and the line reads "Your Mac and local network follow the rules. Reach services on your Mac at host.internal."

## Validation

- **Invalid entry** (scheme, path, port, spaces, malformed address or prefix): field outlined in `bad`, message below in `bad` 13 px: "Enter a domain like pypi.org, an address like 203.0.113.7, or a range like 10.0.0.0/8." The text the user typed stays in the field.
- **Conflict:** the allowed row shows a `warn` line with an arrows-cross icon. Conflicts are allowed to save; they are reported, not blocked.
  - Same domain, or a denied parent domain that includes subdomains: "Also denied. Deny rules win, so this domain stays blocked."
  - Overlapping address ranges: "Overlaps a denied range. Deny rules win where they overlap."
  - Domains and addresses are not compared with each other, since a domain's addresses are only known from DNS.
- **Duplicate:** the input refuses the duplicate and says "Already in this list."

## Advanced

Resolved addresses and their expiry are not shown in the normal flow. They may appear in a diagnostics view later.

## Open decision

Subdomain matching is a per-rule setting, not wildcard syntax (`*.github.com`). Confirm with users; if wildcards are added later, they must map onto the same two scopes.
