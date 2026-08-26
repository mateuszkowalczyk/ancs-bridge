---
name: pmd-update
description: Update an existing Plain Markdown Development installation to the latest or a user-selected distribution version while preserving project documentation and non-PMD instructions. Use when the user asks to update, upgrade, or synchronize PMD. Does not initialize a new installation; use pmd-setup instead.
---

# Update PMD

Synchronize an existing PMD installation with a trusted PMD source without changing the consumer repository's product documentation.

## Managed scope

PMD update may change only:

- PMD-owned directories matching `.agents/skills/pmd-*`
- complete PMD instruction blocks between `<!-- PMD:START -->` and `<!-- PMD:END -->` in repository-root `AGENTS.md` and `CLAUDE.md`

Preserve:

- all content outside the PMD markers
- `docs/` and all other project files
- skills not owned by PMD
- the existing choice of which instruction files contain PMD

## Update source

Use a source directory or version supplied by the user. Otherwise obtain the current default branch of:

`https://github.com/mateuszkowalczyk/plain-markdown-development`

Fetch remote content into a temporary location before changing the target repository. When the source is a Git checkout, record its commit. Never execute scripts from an unverified update source.

The source must contain:

- `.agents/skills/pmd-setup/references/agents-instructions.md` with exactly one complete PMD marker block
- valid `.agents/skills/pmd-*/SKILL.md` files whose frontmatter names match their directory names
- the core `pmd-setup`, `pmd-update`, `pmd-plan`, `pmd-implement`, and `pmd-complete` skills

Stop without modifying the target if source validation fails.

## Procedure

1. Inspect `.agents/skills/`, repository-root `AGENTS.md`, and repository-root `CLAUDE.md` to confirm PMD is already installed. If neither a PMD skill nor a complete PMD instruction block exists, stop and recommend `pmd-setup`.
2. Resolve and validate the update source as described above.
3. Inspect the target for incomplete PMD markers, duplicate PMD blocks, and uncommitted changes that overlap PMD-managed files.
4. If either instruction file contains a single unmatched marker or duplicate PMD blocks, stop before changing any file and ask the user how to resolve it.
5. If PMD-managed files have uncommitted local changes that the update would overwrite, show the affected files and ask for confirmation before continuing. When version control cannot distinguish local customizations from version differences, explain that limitation and ask before overwriting divergent PMD-managed files. Do not treat ordinary committed differences from the new distribution as local modifications.
6. Compare the source and installed PMD skill directories, then:
   - add PMD skills introduced by the source
   - replace each installed PMD skill that is present in the source as a complete directory so files removed upstream do not remain stale
   - preserve all non-PMD skill directories
   - when an installed `pmd-*` skill is absent from the source, report it and ask before deleting it
7. In every repository-root instruction file that already contains one complete PMD block, replace exactly that marked range with the block from the source template. Do not add PMD to an instruction file that does not already contain it.
8. Do not create, edit, move, or delete anything under `docs/`.
9. Verify after the update that:
   - every source PMD skill directory matches the installed copy
   - every updated instruction block matches the source template
   - content outside the markers is unchanged
   - unrelated skills and project files are unchanged
10. Report:
   - the source path, version, or Git commit used
   - PMD skills added, updated, removed, or deliberately retained
   - instruction files whose PMD blocks were updated
   - conflicts, local modifications, or malformed markers left unresolved
11. Recommend restarting the agent or starting a new session so it reloads the updated skills and instructions.

## Safety

An update request authorizes replacing PMD-managed files with the selected distribution, but not discarding detected uncommitted customizations or deleting ambiguous PMD-prefixed skills. Ask before either action.

Stage and validate the complete replacement before applying it when possible. If an update fails partway through, restore the prior PMD-managed files when they were backed up safely; otherwise stop and report the exact partial state rather than attempting speculative repairs.
