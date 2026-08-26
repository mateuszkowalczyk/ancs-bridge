---
name: pmd-setup
description: Install Plain Markdown Development in a repository that does not already have it. Use when the user asks to set up, initialize, or install PMD. Creates the documentation structure and safely inserts the PMD instruction block into the user's chosen AGENTS.md, CLAUDE.md, or both. Use pmd-update for an existing installation.
---

# Set up PMD

Initialize PMD without replacing the user's existing agent instructions.

## Required templates

Read:

`references/agents-instructions.md`

`references/inbox.md`

The first contains the complete PMD instruction block between:

```text
<!-- PMD:START -->
<!-- PMD:END -->
```

## Procedure

1. Inspect the repository structure and its existing `AGENTS.md` and `CLAUDE.md`, if present.
2. Ask whether to install the PMD instructions in `AGENTS.md`, `CLAUDE.md`, or both:
   - use the question tool when it is available
   - otherwise ask the user directly
   - installing in both supports teams whose agents use different instruction-file conventions
   - one file may instead contain the PMD block and the other may explicitly direct agents to read it
   - do not modify either instruction file until the user chooses
   - after the user chooses, inspect both files for PMD markers
3. Create missing directories:

```text
docs/prd/
docs/specs/
docs/tasks/current/
docs/tasks/archived/
```

4. Create missing files:

```text
docs/inbox.md
docs/changelog.md
```

Create `docs/inbox.md` from `references/inbox.md`.

Use this minimal content for `docs/changelog.md`:

```markdown
# Changelog
```

5. Install the PMD instructions into each selected repository-root instruction file:

   - If a selected file does not exist, create it with the complete marked block from `references/agents-instructions.md`.
   - If a selected file exists and has no PMD markers, append the complete marked block after the existing content.
   - If both PMD markers already exist, leave the block unchanged, report that PMD is already installed in that file, and recommend `pmd-update` when the user wants to update it.
   - Preserve all content outside the PMD markers.
   - Never create a second PMD block in either file.
   - Never replace the user's entire instruction file.
   - When only one file is selected, leave the other instruction file unchanged.

6. When only one instruction file is selected and the other contains a complete PMD block:
   - leave it unchanged
   - report that its PMD block may be stale or may intentionally support another agent
   - treat the notice as informational, not as an unresolved conflict
7. If only one PMD marker exists in any selected file, do not guess how to repair it. Report the malformed block and ask the user before editing that file.
8. If the repository already has overlapping requirement or documentation directories:
   - preserve them
   - explain the overlap
   - ask before moving or renaming existing content
9. Report:
   - directories and files created
   - which instruction file or files were selected
   - whether each selected PMD block was inserted or already present
   - any potentially stale PMD block found in the unselected instruction file
   - any conflicts requiring user input
10. After setup finishes without unresolved conflicts, recommend this sequence:
   1. restart the agent or start a new session so it reloads the selected instruction file
   2. briefly describe the app requirements in a Markdown file under `docs/prd/`
   3. use `pmd-plan` to plan the first or next iteration from those requirements

Do not create product requirements, specs, iterations, tasks, or changelog entries during setup unless explicitly asked.

## Idempotency

Running setup repeatedly must be safe:

```text
first run   → create structure and insert the PMD block in the selected instruction file or files
later run   → preserve the existing PMD block and recommend pmd-update
```
