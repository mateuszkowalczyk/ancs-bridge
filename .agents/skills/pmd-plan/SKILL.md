---
name: pmd-plan
description: Plan a new Markdown iteration from selected PRDs, specs, inbox items, bugs, or technical work. Use when the user asks to start, create, scope, or plan an iteration. During planning, discuss unclear product behaviour with the user and draft any needed specs, but obtain explicit spec-change approval before changing spec files. Does not implement code.
---

# Plan an iteration

Create an iteration containing only the scope selected by the user. The agent normally creates the required specs as part of this planning process; the user does not need to write them first.

## Procedure

1. Inspect `docs/tasks/current/` and `docs/tasks/archived/` to determine the next iteration number.
2. Read the PRDs, specs, inbox entries, bug context, and code relevant to the requested scope.
3. Decide whether each item needs a spec:
   - meaningful product behaviour should normally have a spec
   - small bugs, maintenance, refactors, research, and technical work may go directly into an iteration
4. When a required spec is missing or unclear:
   - discuss the intended behaviour with the user during the planning session
   - use the discussion, product requirements, and repository context to draft the spec
   - present the proposed spec content or change
   - obtain explicit spec-change approval from the user before modifying any file in `docs/specs/`
5. Convert the selected scope into small, independently verifiable checkboxes.
6. Group checkboxes in whichever way best communicates the work:
   - by feature
   - by subsystem
   - by work type
   - by another sensible structure
7. Place bugs either in a separate section or beside the feature they affect, whichever is clearer.
8. Create:

   `docs/tasks/current/iteration-NNN-short-name.md`

9. Set the status to `Planned`.
10. Remove an item from `docs/inbox.md` only after it is fully represented in a spec or the iteration.
11. Summarize the iteration and any blockers.
12. Suggest using `pmd-implement` to implement the planned iteration.

Do not implement code.

Spec-change approval permits the proposed spec mutation. It is separate from iteration-completion approval, which is requested and handled by `pmd-complete`.

## Minimal iteration template

```markdown
# Iteration NNN — <Short descriptive name>

**Status:** Planned

## Sources

- `docs/specs/<spec>.md`
- `docs/prd/<prd>.md`

## Tasks

### <Sensible group>

- [ ] <Small, verifiable task>
- [ ] <Small, verifiable task>
```

`Sources` may be omitted when no document reference is useful. Add dependencies, deferred work, open questions, or notes only when they provide real value. Every checkbox is required for iteration completion, so deferred work must use plain list items rather than checkboxes.

## Readiness test

An iteration is ready when:

- every task is understandable and verifiable
- meaningful feature behaviour is sufficiently specified
- blockers are visible
- tasks are reasonably small
- selected scope is distinguishable from deferred work

An iteration does not need a single unifying goal.
