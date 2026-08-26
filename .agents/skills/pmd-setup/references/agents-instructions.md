<!-- PMD:START -->

# PRD-driven development workflow

## Documentation structure

```text
docs/
├── inbox.md
├── changelog.md
├── prd/
├── specs/
└── tasks/
    ├── current/
    └── archived/
```

Development follows this progression:

`PRD → Spec → Iteration tasks → Implementation → Simplification → User validation → Approval → Archived iteration`

The progression is incremental. A whole PRD does not need to be converted into specs or tasks at once.

## Document roles

### PRDs

PRDs live in `docs/prd/`.

They describe product intent, user problems, major features, business requirements, constraints, and high-level acceptance criteria.

PRDs are not implementation plans. Update a PRD only when intended product requirements change.

### Specs

Specs live in `docs/specs/`.

Each spec describes one coherent product capability or system behaviour. Specs describe the currently expected behaviour, not implementation history.

Specs are usually drafted by the agent during iteration planning, based on the product requirements, repository context, and discussion with the user. The user does not need to prepare specs manually.

Specs are required for meaningful product behaviour, but not for every small bug, research task, refactor, or maintenance item.

The agent should discuss unclear behaviour with the user and propose the resulting spec. In any workflow, always obtain explicit spec-change approval from the user before creating, editing, renaming, or deleting a spec.

### Inbox

`docs/inbox.md` is a flat list of loose ideas, bugs, observations, questions, risks, and possible future work.

Inbox entries are possibilities, not requirements or commitments. Do not automatically convert or expand them unless asked.

Remove an inbox entry only after it has been fully represented elsewhere or explicitly rejected.

### Iterations

Active iteration files live in `docs/tasks/current/`.

Completed iteration files live in `docs/tasks/archived/`.

Multiple current iterations may exist at the same time.

Each iteration uses:

`iteration-NNN-short-name.md`

An iteration can contain related or unrelated features, parts of larger features, bugs, technical work, or research.

Task checkboxes may be grouped by feature, work type, subsystem, or any other structure that makes the iteration easy to understand. Bugs may be kept separately or grouped with the affected feature.

Iteration statuses are:

- `Planned`
- `Awaiting approval`
- `Completed`

Checkboxes are:

- `[ ]` — incomplete
- `[x]` — complete

Every checkbox in an iteration is required for completion. Do not use checkboxes for deferred work or custom checkbox states.

Iteration files require only:

- title
- status
- sources when applicable
- task checkboxes

Add sections such as dependencies, deferred work, open questions, or completion notes only when useful. Record deferred work as plain list items, not checkboxes.

### Changelog

User-visible completed outcomes live in `docs/changelog.md`.

Create or update the changelog only once, after the user approves completion of an iteration following the latest readiness report. The request that starts a completion review is not archive approval, even if it asks to approve or archive.

Every changelog entry must reference the archived iteration that delivered it. Do not add planned, speculative, or unfinished work.

## Completion rules

A task is complete only when:

- implementation is finished
- relevant automated tests, linters, type checks, build checks, agent-performed validation, and other checks pass both before and after simplification
- the implementation and directly related code have been reviewed for avoidable complexity, with safe simplifications applied
- any meaningful simplification tradeoff has been decided by the user
- any required user-run manual validation passes
- acceptance criteria are satisfied
- affected documentation is consistent

Perform simplification after the agent has implemented and validated the work, but before asking the user to run manual tests or presenting the iteration for acceptance. Rerun the relevant checks after simplification even when it produces no code changes. Keep the review related to the selected implementation rather than expanding it into unrelated cleanup. Do not silently choose a simplification that trades off behaviour, scope, compatibility, performance, maintainability, or risk; present viable options with concise pros and cons and wait for the user's decision.

Whenever user review or manual validation leads to implementation changes, run the relevant checks, repeat the simplification review, rerun the checks, and only then return the revised work for further user validation or acceptance. Repeat this cycle after every such change.

Do not update a spec merely because implementation was completed. A spec should change only when expected behaviour must be clarified or changed, and only after explicit spec-change approval from the user. If user suggests something different from the spec at some point, ask them if you should change the spec to match that.

Iteration-completion approval is a separate gate handled by `pmd-complete`. An iteration must not be marked `Completed`, added to the changelog, or archived without that approval.

An iteration must not be archived with any unchecked checkbox. A task intentionally removed from scope must be moved to a clearly labeled deferred section as a plain list item or to `docs/inbox.md`.

Before making any completion changes, verify that the iteration has status `Awaiting approval`, still exists only in `docs/tasks/current/`, and has no changelog entry or archived-path reference. If any check fails, stop without changing files; never overwrite an archived iteration or duplicate its changelog entry.

## Source-of-truth order

When documents conflict, use this priority:

1. Explicit current user instruction
2. PRD
3. Spec
4. Current iteration file
5. Existing implementation
6. Inbox entry

Do not silently resolve meaningful contradictions. Explain them and update the appropriate source.

## Workflow skills

Use the relevant project skill for procedural work:

- `pmd-setup` — initialize the PMD folder and file structure
- `pmd-update` — update an existing PMD installation while preserving project documentation and non-PMD instructions
- `pmd-plan` — discuss the scope, draft any needed specs, and plan an iteration
- `pmd-implement` — implement all or part of an iteration
- `pmd-complete` — verify, request approval, update changelog, and archive an iteration

After finishing a skill, suggest the appropriate next workflow step:

- setup → restart the agent or start a new session, briefly describe the app requirements in `docs/prd/`, then use `pmd-plan`
- update → restart the agent or start a new session so it reloads the updated PMD skills and instructions
- planning → `pmd-implement`
- implementation with behaviour not covered by or inconsistent with current specs → resolve the gap before continuing by fixing the implementation, or by discussing and drafting the needed spec addition or update and obtaining explicit spec-change approval
- partial implementation → `pmd-implement` for another part
- full implementation → `pmd-complete`
- completion → `pmd-implement` for another current iteration, or `pmd-plan` when new planning is needed

<!-- PMD:END -->
