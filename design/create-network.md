# Create: network (step 3 of 4)

Third creation step. Flow rules are in [create-agent.md](create-agent.md#creation-flow-shared-by-all-four-steps). The same editor is reused in Settings ([settings-pending.md](settings-pending.md)). Policy semantics come from [ARCHITECTURE.md](../ARCHITECTURE.md#network-policy).

## Layout

```
Domains not listed below                                   │ How rules match
[Deny|Allow]  The agent can reach only the domains you allow.│ github.com matches that exact
                                                            │ name. Turn on subdomains to
✓ Allowed                       ⊘ Denied                    │ include api.github.com…
api.anthropic.com  Exact name ×  gist.github.com Exact name × │
github.com   Includes subdomains × pastebin.com Incl. subdom. ×│ When a domain is both allowed
crates.io    Includes subdomains × [Add a domain to deny]    │ and denied, it's denied.
gist.github.com    Exact name                               │
 ⇄ Also denied below. Deny rules win, so this domain        │ Domain rules use DNS results…
   stays blocked.                                           │
[https://pypi.org/simple]  (red outline)                    │ ⌂ Your Mac and local network
 Enter a domain only, like pypi.org.                        │   stay unreachable, whatever
                                                            │   the rules say.
Cancel                                        [< Back]  [Continue to review]
```

- Main column plus a 280 px help column separated by a 1 px `line` border, 24 px padding.

## Elements

- **Default action:** heading "Domains not listed below" with a segmented control "Deny | Allow" (Deny is the default) and one explanatory sentence that changes with the choice:
  - Deny: "The agent can reach only the domains you allow."
  - Allow: "The agent can reach any domain except the ones you deny."
- **Allowed** and **Denied** lists side by side, each headed by an icon (`ok` circle-check, `bad` ban) and the word. Each rule row: domain (monospace), scope text in `tx2` 12 px ("Exact name" or "Includes subdomains", toggled by clicking), remove icon. An input at the bottom of each list ("Add a domain to allow" / "Add a domain to deny").
- **Help column** ("How rules match"):
  - "github.com matches that exact name. Turn on subdomains to include api.github.com and others."
  - "When a domain is both allowed and denied, it's denied."
  - "Domain rules use DNS results to control destination addresses. A recorded address may be associated with more than one domain."
  - With a home-off icon: "Your Mac and local network stay unreachable, whatever the rules say."

## Validation

- **Invalid entry** (scheme, path, port, spaces, IP address): field outlined in `bad`, message below in `bad` 13 px: "Enter a domain only, like pypi.org." The text the user typed stays in the field.
- **Conflict** (same domain or covered subdomain in both lists): the allowed row shows a `warn` line with an arrows-cross icon: "Also denied below. Deny rules win, so this domain stays blocked." Conflicts are allowed to save; they are reported, not blocked.
- **Duplicate:** the input refuses the duplicate and says "Already in this list."

## Advanced

Resolved addresses and their expiry are not shown in the normal flow. They may appear in a diagnostics view later.

## Open decision

Subdomain matching is a per-rule setting, not wildcard syntax (`*.github.com`). Confirm with users; if wildcards are added later, they must map onto the same two scopes.
