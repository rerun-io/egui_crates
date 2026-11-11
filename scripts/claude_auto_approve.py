#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
from typing import Any

ORG = "rerun-io"
REPO = f"{ORG}/egui_crates"
BASE_BRANCHES = re.compile(r"main")

MARKER = re.compile(r"<!-- rerun-bot-auto-approve sha=([0-9a-f]{40}) -->")

SHARED_ACCOUNTS = {"rerun-bot"}

MAX_CHANGED_LINES = 3000
MAX_CHANGED_FILES = 75

PROTECTED_PATHS = [
    re.compile(r"^\.github/workflows/claude_auto_approve\.yml$"),
    re.compile(r"^scripts/claude_auto_approve\.py$"),
]


def marker_sha(comment_body: str) -> str | None:
    shas = set(MARKER.findall(comment_body))
    return shas.pop() if len(shas) == 1 else None


def protected(path: str) -> bool:
    return any(pattern.search(path) for pattern in PROTECTED_PATHS)


def check(pr: dict[str, Any], files: list[dict[str, Any]], sha: str, author_is_member: bool) -> list[str]:
    problems = []

    if pr["state"] != "open":
        problems.append(f"The pull request is {pr['state']}.")
    if pr["draft"]:
        problems.append("The pull request is a draft.")
    if not BASE_BRANCHES.fullmatch(pr["base"]["ref"]):
        problems.append(f"The base branch is `{pr['base']['ref']}`, not `main`.")
    if (pr["head"]["repo"] or {}).get("full_name") != REPO:
        problems.append("The head branch is in a fork.")
    if pr["head"]["sha"] != sha:
        problems.append(f"The head is now {pr['head']['sha']}, not the reviewed {sha}.")

    author = pr["user"]
    if author["type"] != "User" or author["login"] in SHARED_ACCOUNTS:
        problems.append(f"The author `{author['login']}` is not a person.")
    elif not author_is_member:
        problems.append(f"The author `{author['login']}` is not a member of the organization.")

    if pr["changed_files"] != len(files):
        problems.append(f"Only {len(files)} of {pr['changed_files']} changed files could be listed.")
    if pr["changed_files"] > MAX_CHANGED_FILES:
        problems.append(f"{pr['changed_files']} files changed; the limit is {MAX_CHANGED_FILES}.")
    changed_lines = pr["additions"] + pr["deletions"]
    if changed_lines > MAX_CHANGED_LINES:
        problems.append(f"{changed_lines} lines changed; the limit is {MAX_CHANGED_LINES}.")

    for file in files:
        paths = [file["filename"]] + ([file["previous_filename"]] if "previous_filename" in file else [])
        for path in paths:
            if protected(path):
                problems.append(f"`{path}` is a protected path.")

    return problems


def gh_api(*args: str) -> Any:
    out = subprocess.run(["gh", "api", *args], check=True, capture_output=True, text=True).stdout
    return json.loads(out)


def is_org_member(login: str) -> bool:
    env = {**os.environ, "GH_TOKEN": os.environ["ORG_MEMBERS_TOKEN"]}
    result = subprocess.run(
        ["gh", "api", "--silent", f"/orgs/{ORG}/members/{login}"], env=env, capture_output=True, check=False
    )
    return result.returncode == 0


def gh_set_output(key: str, text: str) -> None:
    output_file = os.environ.get("GITHUB_OUTPUT")
    if output_file is not None:
        with open(output_file, "a", encoding="utf8") as f:
            f.write(f"{key}={text}\n")
    else:
        print(f"{key}={text}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pr-number", required=True, type=int)
    parser.add_argument("--report", required=True, help="File to write refusal reasons to")
    args = parser.parse_args()

    sha = marker_sha(os.environ.get("GITHUB_COMMENT_BODY", ""))
    if sha is None:
        refuse(args.report, ["Missing or ambiguous approval marker."])
        return

    pr = gh_api(f"/repos/{REPO}/pulls/{args.pr_number}")
    pages = gh_api("--paginate", "--slurp", f"/repos/{REPO}/pulls/{args.pr_number}/files?per_page=100")
    problems = check(
        pr=pr,
        files=[file for page in pages for file in page],
        sha=sha,
        author_is_member=is_org_member(pr["user"]["login"]),
    )
    if problems:
        refuse(args.report, problems)
        return

    print(f"All gates passed for {sha}")
    gh_set_output("sha", sha)
    gh_set_output("base", pr["base"]["ref"])


def refuse(report: str, problems: list[str]) -> None:
    pathlib.Path(report).write_text("\n".join(f"- {problem}" for problem in problems) + "\n", encoding="utf8")
    print("\n".join(problems), file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()
