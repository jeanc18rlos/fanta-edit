---
name: rules-harvest
description: Harvest the "Suggested .rules additions" sections of merged pull requests into the root rules and crate AGENTS.md files, verify each suggestion against current code, and open one dedicated pull request. Use when asked to harvest, sync or update agent rules from pull requests, or on a scheduled rules-harvest run.
---

# Harvesting suggested rules

Crate-level traps normally land in the pull request that found them. This harvest picks up root-level suggestions and anything that slipped through. Read the "Rules hygiene" section of the root `.rules` first; its bar applies here.

## Steps

1. Find the watermark. The most recent merged harvest pull request ends its body with `Harvested through #N`:
   `gh pr list --state merged --search "Harvest suggested rules in:title" --limit 1 --json number,body`
   With no earlier harvest, start from the first pull request.
2. Collect candidates: merged pull requests numbered above `N` whose body contains "Suggested .rules additions".
   `gh pr list --state merged --limit 200 --json number,title,body`
3. For each suggestion:
   - Pick its home: the root `.rules` for cross-crate traps and process, a crate `AGENTS.md` for one crate's traps, or a skill for workflow know-how (benchmarks, releases, QA).
   - Check it against current code. The functions, types and files it names must exist and still behave as described; read them. Rewrite stale suggestions or drop them.
   - Merge duplicates. A trap that recurred across pull requests is a strong keep.
   - Write it as an imperative bullet that states the consequence, and cite the source: `(#123)`.
4. While reading, delete existing rules whose code is gone.
5. Keep files lean: the root `.rules` under about 200 lines, crate files grouped by topic.
6. For a new crate file, create `crates/<crate>/AGENTS.md`, add `ln -s AGENTS.md CLAUDE.md` beside it, and list it under "Crate rules" in the root `.rules`.
7. Open one pull request titled `Harvest suggested rules from #A–#B`. In the body, list each harvested, rewritten and dropped suggestion with its source, end the list with `Harvested through #B`, and finish with `Release Notes:` and `- N/A`.

Do not merge the pull request yourself.
