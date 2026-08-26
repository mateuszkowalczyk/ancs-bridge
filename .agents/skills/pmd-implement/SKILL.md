---
name: pmd-implement
description: Implement, validate, and simplify all or a user-selected part of a current Markdown iteration while keeping task checkboxes and documentation consistent. Use when the user asks to implement an iteration, a task group, specific tasks, or the next unchecked task. May prepare the iteration for approval, but never archives it.
---

# Implement iteration tasks

Implement only the portion of an iteration requested by the user.

## Supported scopes

The user may ask to implement:

- the entire iteration
- one task group
- specific named tasks
- the next unchecked task
- all remaining tasks
- any other clearly identified subset

Do not expand the requested scope without explaining why it is necessary.

## Procedure

1. Locate the relevant file in `docs/tasks/current/`.
2. Read all referenced specs and relevant PRD sections before changing code.
3. Select tasks strictly according to the user's instruction.
4. Implement the smallest coherent set of changes needed.
5. Add or update tests that verify the acceptance criteria.
6. Run relevant automated tests, linters, type checks, build checks, and any other validation the agent can perform directly.
7. Perform the simplification review described below. After the review and any resulting changes, rerun the relevant automated tests, linters, type checks, build checks, and agent-performed validation.
8. Only after the simplification review is resolved, ask the user to perform any validation that requires their manual action.
9. If user review or manual validation leads to implementation changes, run the validation and simplification cycle again before returning the revised work to the user.
10. Mark a checkbox `[x]` only when:
   - implementation is complete
   - relevant checks pass
   - acceptance criteria are met
   - the simplification review is resolved
   - affected documentation is consistent
11. Leave incomplete and unselected tasks unchecked.
12. Add concise implementation notes only when useful.
13. Do not update a spec merely because implementation is complete.
14. If expected behaviour needs to be clarified or changed:
   - discuss the behaviour with the user
   - draft the proposed spec update from that discussion and repository context
   - obtain explicit spec-change approval from the user
   - only then modify the spec
15. Do not update `docs/changelog.md`.
16. Report:
   - tasks completed
   - tasks still open
   - checks run and results
   - simplifications made or tradeoffs awaiting a decision
   - manual validation still needed from the user
   - deviations, blockers, or proposed documentation changes
17. Suggest the next step:
   - when any implemented behaviour is not covered by the current specs or is inconsistent with them, call out the gap and suggest resolving it before continuing: fix the implementation when the spec remains authoritative, or discuss and draft the needed spec addition or update when the implemented behaviour is intended; obtain explicit spec-change approval before modifying any spec
   - when the requested work covered only part of the iteration, suggest using `pmd-implement` to implement another part
   - when any checkbox remains unchecked, suggest using `pmd-implement` to continue it
   - when all checkboxes are checked and validation passes, tell the user: "Once you're ready to approve and archive the iteration, run `pmd-complete`."

## Simplification review

Run this review after implementation and all validation the agent can perform directly, but before user-run manual validation and before presenting the work for acceptance or completion review.

Inspect the selected implementation and the directly related code for avoidable complexity, including:

- an implementation that is more complicated than the requirement needs
- duplication, indirection, or abstractions that do not earn their cost
- related code that can be refactored to make the solution smaller or clearer
- defensive handling for edge cases that cannot realistically occur
- a different design that would materially simplify the implementation

Apply clear, behaviour-preserving simplifications that remain within the selected scope. Do not turn this review into unrelated cleanup.

When simplification would require a meaningful tradeoff in behaviour, scope, compatibility, performance, maintainability, or risk, do not choose silently. Present the user with the viable options, including keeping the current implementation, and give concise pros and cons for each. Wait for the user's decision before proceeding to user-run manual validation or acceptance. A choice to keep the current implementation resolves the review.

Rerun the relevant automated tests, linters, type checks, build checks, and agent-performed validation after the simplification review, even when the review produces no code changes. This creates distinct validation checkpoints before and after simplification.

Treat changes requested during user review or prompted by failed manual validation as another implementation pass: make the changes, run the relevant checks, repeat the simplification review, rerun the checks, and only then return the revised work for further user validation or acceptance. Repeat this cycle after every such change.

If simplification reveals that expected behaviour or a spec should change, follow the explicit spec-change approval procedure.

## Preparing completion automatically

After implementation, inspect the whole iteration.

When all checkboxes are checked and validation passes:

1. add concise completion notes only when useful
2. set the iteration status to `Awaiting approval`
3. present a completion summary
4. tell the user: "Once you're ready to approve and archive the iteration, run `pmd-complete`."

Explicit spec-change approval is still required before modifying a spec during implementation. Do not request iteration-completion approval, including approval to archive, during implementation; that separate gate belongs to `pmd-complete`. Do not archive the iteration, update the changelog, or mark it `Completed`.

## Scope discipline

Do not opportunistically implement unrelated inbox items or deferred work.

Every checkbox is required for iteration completion. If a task is no longer needed, move it to a clearly labeled deferred section as a plain list item or to `docs/inbox.md`.

When implementation reveals new work:

- add a required blocker to the current iteration only when necessary to complete selected scope
- otherwise append a concise item to `docs/inbox.md`
- explain the decision
