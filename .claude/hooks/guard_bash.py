#!/usr/bin/env python3
"""PreToolUse guard for Bash commands run by Claude Code in this repository.

It denies two commands that cost agents the most time in past sessions:

* `./script/clippy` without `-p`/`--package`, which runs a full-workspace
  release clippy (over 20 minutes). `FANTA_FULL_CLIPPY=1` in the command
  allows a full run when the user asked for one.
* From a linked worktree, pointing `CARGO_TARGET_DIR` or `--target-dir` at the
  main checkout's `target/`. The user's editor builds there, so the agent ends
  up waiting on Cargo's lock.

Any unexpected error lets the command through: a broken guard must never
block work.
"""

import json
import os
import re
import shlex
import subprocess
import sys

FULL_CLIPPY_OVERRIDE = "FANTA_FULL_CLIPPY=1"
SHELLS = {"bash", "sh", "zsh"}
COMMAND_PREFIXES = {"env", "time", "nohup", "exec", "command"}
SEGMENT_SEPARATOR = re.compile(r"&&|\|\||[;|&\n()`]|\$\(")
ENVIRONMENT_ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
TARGET_DIRECTORY_SETTING = re.compile(
    r"(?:^|[\s;&|(])(?:export\s+)?CARGO_TARGET_DIR=(\"[^\"]*\"|'[^']*'|[^\s;&|)]+)"
    r"|--target-dir(?:=|\s+)(\"[^\"]*\"|'[^']*'|[^\s;&|)]+)"
)


def deny(reason):
    json.dump(
        {
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        },
        sys.stdout,
    )


def split_words(segment):
    try:
        return shlex.split(segment)
    except ValueError:
        return segment.split()


def runs_unscoped_clippy(command):
    if FULL_CLIPPY_OVERRIDE in command:
        return False
    for segment in SEGMENT_SEPARATOR.split(command):
        words = split_words(segment)
        while words and (ENVIRONMENT_ASSIGNMENT.match(words[0]) or words[0] in COMMAND_PREFIXES):
            words = words[1:]
        if words and os.path.basename(words[0]) in SHELLS:
            options = []
            words = words[1:]
            while words and words[0].startswith("-"):
                options.append(words.pop(0))
            # `bash -n script/clippy` only checks syntax.
            if "-n" in options:
                continue
        if not words or not words[0].endswith("script/clippy"):
            continue
        arguments = words[1:]
        scoped = any(
            argument == "-p"
            or argument.startswith("--package")
            or (argument.startswith("-p") and not argument.startswith("--"))
            for argument in arguments
        )
        if not scoped:
            return True
    return False


def git_path(cwd, *arguments):
    result = subprocess.run(
        ["git", "-C", cwd, "rev-parse", *arguments],
        capture_output=True,
        text=True,
        timeout=5,
    )
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def main_checkout_target(cwd):
    common_directory = git_path(cwd, "--path-format=absolute", "--git-common-dir")
    top_level = git_path(cwd, "--show-toplevel")
    if not common_directory or not top_level:
        return None
    if os.path.basename(common_directory) != ".git":
        return None
    main_checkout = os.path.dirname(common_directory)
    if os.path.realpath(top_level) == os.path.realpath(main_checkout):
        return None
    return os.path.realpath(os.path.join(main_checkout, "target"))


def resolve(value, cwd):
    value = value.strip("\"'")
    home = os.path.expanduser("~")
    value = os.path.expanduser(value.replace("${HOME}", home).replace("$HOME", home))
    return os.path.realpath(os.path.join(cwd, value))


def shares_main_checkout_target(command, cwd):
    if "CARGO_TARGET_DIR=" not in command and "--target-dir" not in command:
        return None
    main_target = main_checkout_target(cwd)
    if main_target is None:
        return None
    for match in TARGET_DIRECTORY_SETTING.finditer(command):
        value = match.group(1) or match.group(2)
        if value and resolve(value, cwd) == main_target:
            return main_target
    return None


def main():
    payload = json.load(sys.stdin)
    if payload.get("tool_name") != "Bash":
        return
    command = (payload.get("tool_input") or {}).get("command") or ""
    cwd = payload.get("cwd") or os.getcwd()

    if "clippy" in command and runs_unscoped_clippy(command):
        deny(
            "Run `./script/clippy -p <crate>` for the crates you changed. Without `-p` it "
            "runs a full-workspace release clippy that takes over 20 minutes. If the user "
            f"asked for the full run, prefix the command with `{FULL_CLIPPY_OVERRIDE}`."
        )
        return

    main_target = shares_main_checkout_target(command, cwd)
    if main_target:
        deny(
            f"Do not build into {main_target}: it is the main checkout's target directory, "
            "where the user's editor builds, so you would wait on Cargo's lock. Use this "
            "worktree's own target/ (Cargo's default) instead."
        )


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"guard_bash.py: {error}", file=sys.stderr)
        sys.exit(0)
