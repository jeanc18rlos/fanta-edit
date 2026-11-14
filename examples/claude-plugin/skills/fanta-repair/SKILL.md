---
name: fanta-repair
description: Repair a Fanta project whose files were damaged outside the app — a merge conflict, a deleted or hand-edited file, a broken reference. Runs the project check, proposes the smallest file changes that clear its findings, and checks again. Use when check_project or fanta-check reports findings, when Fanta says a design is unavailable, or after a git merge or manual edit of a Fanta project.
---

# Repairing a Fanta project

A Fanta project is a folder of plain files (`fanta.json`, `pages/`,
`components/`, `doc/`, `assets/`). The check lists everything wrong with
those files. Your job is to make the check clean again with the smallest
change, and to change nothing the check did not name.

## 1. Run the check

Call the `check_project` tool. Without it, run
`fanta-check <project> --json`. Both return a JSON array of findings:
`severity`, `code`, `file`, `line`, `column`, `node_id`, `message`. An empty
array means there is nothing to repair. Warnings count: `fanta-check` exits
0 when there are only warnings, so read the array, not the exit code. If you
have neither, stop and ask
the user to open the project in Fanta, which shows the findings.

## 2. Find the last good version

Skip this step when the repair needs no older version.

There are two sources, and you read only the first yourself:

- The user's own git repository, when the project is in one:
  `git log -- <file>` and `git show <commit>:<file>` there. Take the newest
  commit in which the damaged file was still good. Check a candidate by
  reading it, not by its date.
- Fanta's snapshots. Fanta records the project before and after every save
  in `.fanta/history.git`, and the user restores one in the app with
  File > Restore Snapshot. A restore puts back every file of the project as
  it was at that time, and records the current state first, so it can be
  undone the same way.

Never run `git` with `.fanta/history.git` as its `--git-dir` or `GIT_DIR`,
not even to list or read, and never read or change files inside `.fanta/`.
That directory comes with the project folder, so in a project from someone
else its settings can make any git command, `diff` and `log` included, run
programs on this machine. Only Fanta opens it.

When the file you need is in no commit, say so and ask the user to restore
a snapshot from before the damage, then run the check again. Tell them that
a restore also takes back every other change made since that snapshot.

## 3. Propose the change

Work finding by finding. Edit only the file a finding names, with two
exceptions: a finding on a design's SVG may be fixed in the stylesheet
beside it (`page.css`, `master.css`), where the node's rule lives; and a
finding that a file or component is gone is fixed by restoring exactly that
file or folder. Ids in JSON files are written without their prefix
(`01M7…`, not `c_01M7…` or `n_01M7…`), so search for the bare form.

| Code | Repair |
| --- | --- |
| `parse.conflict_marker` | Resolve the conflict and remove every marker line. Keep both sides when they change different nodes or properties; when they change the same value, ask the user which one wins. |
| `parse.json`, `parse.css`, `parse.svg` | Fix the syntax error at the reported line. If the file is beyond a local fix, take it from the last good version. |
| `manifest.file_missing`, `ref.missing_asset` (file gone) | Restore the file from a commit of the user's repository: `mkdir -p <its folder>`, then `git show <commit>:<path> > <path>`. An asset is `assets/images/<asset id>.<extension>`; `git ls-tree -r --name-only <commit>` finds it. A missing SVG or stylesheet (a warning) is also rewritten when the project is saved in Fanta, but the committed copy keeps hand edits. For a binary asset use the same command; do not paste its content. If no commit has the file, ask the user to restore a snapshot (step 2). `nodes.json` and images cannot be rebuilt by hand. |
| `ref.missing_component` | Find the component's folder by searching the last good commit's `components/*/def.json` for the id, then restore the folder file by file. If the user says they deleted it on purpose, ask whether the instances should be removed in the app instead. |
| `id.duplicate` | A pasted copy shares its original's `id`. The finding's line is the second element with that id: remove that one, unless the last good version shows the other is the newcomer. To get a real duplicate, copy the node in Fanta, which gives it its own id and record. |
| `id.not_a_node`, `record.no_element` on the same file | An `id` was renamed in the SVG. The element with the unknown id is the one the missing record belongs to: put the node id back. Node ids are in `nodes.json` and in the last good version of the SVG. |
| `id.not_a_node` alone | An element or rule Fanta does not know, usually metadata another application added. Remove it; a save drops it anyway. |
| `record.missing` | An element or rule names a node with no record. Restore the record's owner (see `ref.missing_component`) or remove the orphan element or rule. |
| `ref.missing_token` | The `var()` is in the design's stylesheet. Name the right token only when the last good version or the user tells you which it was (`tokens.css` lists the ones that exist). Otherwise drop the `var()` and keep its fallback value, which changes nothing on screen. |
| `edit.not_kept`, `edit.selector_outside_dialect` | The edit is outside what Fanta reads from the file, and the next save discards it. Move a `style` attribute's properties into the node's rule in the design's stylesheet, replacing the rule's value for the same property, and remove the attribute; rewrite a selector as a single `#<node id>`. If the change cannot be expressed that way, make it in the app. |
| `design.unavailable` | Not repaired itself: fix the `parse.*` or `manifest.file_missing` finding in that folder and it goes away. |
| `snapshot.unavailable`, `load.omitted_geometry`, `ref.ambiguous_token`, `source.unknown_attribute` | Not file damage. Report them to the user and leave the files alone. |

Rules that hold for every repair:

- Never invent a node id, write a `nodes.json` record by hand, or reformat a
  file. Ids and records come from Fanta or from the last good version.
- Prefer the fix that keeps the user's intended change. Restoring a whole
  file from a commit discards every edit made to it since; say what is
  lost when you do it.
- Do not edit generated files (`index.html`, `viewer.js`, `canvas.js`,
  `editor.js`, `pages.js`, `tokens.css`, `report.json`).
- Show the user the change as a diff before it is applied when your
  environment has a review step. In the Fanta app your file edits are shown
  for the user to accept or reject; do not work around that.

## 4. Check again

Run the check again. Repeat from step 3 for findings that remain. Stop when
the array is empty, or when what is left needs the user: a choice between
two sides, a file no commit has, or a change that must be made in the app.
Then report what you changed, file by file, and what you left and why.

If Fanta has the project open, it reloads the files when they change. A
user with unsaved edits in the app should save or discard them first.
