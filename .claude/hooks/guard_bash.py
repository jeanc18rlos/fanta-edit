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
SEGMENT_SEPARATORS = "();<>|&\n`"
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


def split_segments(command):
    """Splits a command line into the word lists of its simple commands.

    Shell operators only separate commands outside quotes, so the line is tokenized first:
    `grep 'a\\|script/clippy' file` is one command whose pattern happens to name the script.
    """
    lexer = shlex.shlex(command, posix=True, punctuation_chars=SEGMENT_SEPARATORS)
    lexer.whitespace = " \t\r"
    lexer.whitespace_split = True
    lexer.commenters = ""
    try:
        tokens = list(lexer)
    except ValueError:
        tokens = command.split()
    segments = [[]]
    for token in tokens:
        if token and all(character in SEGMENT_SEPARATORS for character in token):
            segments.append([])
        else:
            segments[-1].append(token)
    return segments


def runs_unscoped_clippy(command):
    if FULL_CLIPPY_OVERRIDE in command:
        return False
    for words in split_segments(command):
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
            if any(option.endswith("c") for option in options):
                if words and runs_unscoped_clippy(words[0]):
                    return True
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


def self_test():
    denied = [
        "./script/clippy",
        "script/clippy --fix",
        "cd crates && ../script/clippy",
        "FOO=1 time ./script/clippy",
        "ls\n./script/clippy",
        "echo $(./script/clippy)",
        "bash -c './script/clippy && ls'",
    ]
    allowed = [
        "./script/clippy -p fanta-doc",
        "./script/clippy --package fanta-doc",
        "FANTA_FULL_CLIPPY=1 ./script/clippy",
        "bash -n script/clippy",
        "bash -c './script/clippy -p fanta-doc'",
        "grep -rn 'runs_unscoped\\|script/clippy' .claude",
        'grep -E "cargo clippy|script/clippy" docs',
        "grep -rn 'cargo clippy\\|script/clippy\\|cargo fmt' docs",
        "grep -rnE '(script/clippy)( |$)' docs",
        "grep -rn '`./script/clippy`' CLAUDE.md",
        'grep -rn "cargo clippy\\|script/clippy " docs',
        "cat script/clippy",
        "git log -- script/clippy",
    ]
    for command in denied:
        assert runs_unscoped_clippy(command), f"should deny: {command}"
    for command in allowed:
        assert not runs_unscoped_clippy(command), f"should allow: {command}"
    print(f"guard_bash.py: {len(denied) + len(allowed)} cases pass")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        sys.exit(0)
    try:
        main()
    except Exception as error:
        print(f"guard_bash.py: {error}", file=sys.stderr)
        sys.exit(0)
