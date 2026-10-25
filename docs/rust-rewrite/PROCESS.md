# Rust Rewrite Process

How work moves through GitHub issues during the rewrite, and which agent role owns each step.

## Labels

Implementation (every issue gets exactly one):

| Label | Meaning |
| --- | --- |
| `python` | About the current Python/Textual app on `main`. All issues filed before the rewrite carry this. |
| `rust` | About the Rust rewrite on `rust-main`. |

Type (one):

| Label | Meaning |
| --- | --- |
| `bug` | Existing label. Something is broken. |
| `enhancement` | Existing label. New behavior. For `rust` issues this is parked until parity unless Chris says otherwise. |
| `parity` | The Rust app must reproduce a Python behavior. Most `rust` issues are this. |
| `parity-gap` | A difference found between the two apps after something was marked done. |
| `infra` | Build, CI, harness, packaging, tooling. |

Status (one, `rust` issues only; triage owns moving into the first state):

| Label | Owner of the next step |
| --- | --- |
| `status: needs-triage` | Triage agent |
| `status: needs-design` | Design agent |
| `status: needs-decision` | Chris. The issue states the question and a recommendation. |
| `status: ready` | Implementer. Spec and acceptance criteria are in the issue. |
| `status: in-progress` | The implementer assigned to the issue. |
| `status: in-review` | Code reviewer and parity reviewer, on the linked PR. |
| `status: blocked` | Whoever owns the blocking issue, which must be linked. |

Area (any number): `area: shell`, `area: repos`, `area: pull-requests`, `area: issues`, `area: workflows`,
`area: notifications`, `area: settings`, `area: core`, `area: parity-harness`.

Platform (optional): `platform: windows`, `platform: macos`.

Labels are defined in [`labels.json`](./labels.json) and created with `scripts/sync-labels.sh` (requires `gh`).

## Milestones

Labels `milestone: M0` through `milestone: M5` (Foundations, App shell, Read-only browsing, Write actions, Remaining
features, Hardening and cutover), plus one tracking issue per milestone with a checklist of its issues. See
[PLAN.md](./PLAN.md#8-milestones).

## Lifecycle

```
needs-triage ──► needs-design ──► ready ──► in-progress ──► in-review ──► closed
      │               │                          │               │
      └──► needs-decision ◄──────────────────────┴───────────────┘
                 (Chris answers, issue returns to the state it came from)
```

1. **Triage** (triage agent). New issue arrives. Apply implementation, type, area and milestone labels. Close
   duplicates with a link. If it's a `python` bug, leave it for the Python track and don't move it into the rust flow.
   Move `rust` issues to `needs-design`, or straight to `ready` if the issue already meets the spec bar below.
2. **Design** (design agent). Read the Python implementation for the behavior, run the parity harness against the
   Python app to capture reference screens, and write the spec into the issue body under the headings in
   [Spec format](#spec-format). Split anything bigger than about one day of implementation into sub-issues.
   Move to `ready`.
3. **Implement** (implementer). Assign yourself, move to `in-progress`, branch `rust/<n>-<slug>` from `rust-main`,
   implement, add tests and a parity scenario, open a PR into `rust-main` that says `Closes #<n>`. Move to `in-review`.
4. **Review** (code reviewer and parity reviewer, in parallel). The code reviewer checks Rust quality; the parity reviewer
   runs the scenarios and compares against Python. Either can request changes. Both approving is the merge bar.
5. **Merge.** Squash into `rust-main`. The implementer enables auto-merge (squash) on the PR when opening it, so it
   merges on its own once required checks pass and both reviews approve. Chris can also merge by hand. This relies on
   branch protection for `rust-main` that requires the CI checks and approving reviews; without it, GitHub would merge
   as soon as auto-merge is enabled. The parity reviewer
   updates the row in [PARITY.md](./PARITY.md).

Any role that hits a choice that changes scope, UX or a dependency choice sets `status: needs-decision`, writes the
question with options and a recommendation as an issue comment, and stops work on that issue.

## Spec format

Every `ready` issue body has these sections:

- **Python reference**: files and functions in `lazy_github/` that implement the behavior, with line links on `main`.
- **Behavior**: what the user sees and can do, including keybindings by binding id, empty states, loading states and
  errors.
- **Out of scope**: what this issue deliberately doesn't cover, with links to issues that do.
- **Design notes**: the Rust approach (components, state, `Github` trait methods, crates). Not a line-by-line port.
- **Acceptance criteria**: a checklist. Always includes the parity scenario(s) that must pass and the snapshot tests.
- **Fixtures**: which `parity/fixtures` are needed, and whether new ones must be recorded.

## Rules that apply to every agent

- Never push to `main`, and never open a PR into `main`, until the cutover issue says so.
- Never merge a PR by hand, and never approve a PR you wrote. Enabling auto-merge (squash) on a PR into `rust-main` is
  allowed; it never applies to PRs into `main`.
- Don't change the Python code on `rust-main`. Python fixes go to `main` and flow into `rust-main` by merge.
- Keep one issue per PR. If you find extra work, file an issue for it (`status: needs-triage`) and link it.
- Leave a short comment on the issue whenever you change its status, saying why.
