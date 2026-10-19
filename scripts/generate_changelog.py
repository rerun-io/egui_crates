#!/usr/bin/env python3
# Based on https://github.com/rerun-io/rerun_template, changed for this monorepo:
# each published crate has its own version, tags and changelog.

"""
Summarizes the PRs since the last release of one crate.

A PR goes into the changelog of a crate if it has the crate's label,
or if it changes a file in the crate's directory.
PRs that are already in the crate's CHANGELOG.md (e.g. from a patch release) are skipped.

The result can be copy-pasted into CHANGELOG.md,
though it often needs some manual editing too.
"""

# /// script
# requires-python = ">=3.9"
# dependencies = [
#     "requests",
#     "GitPython",
#     "tqdm",
# ]
# ///

from __future__ import annotations

import argparse
import multiprocessing
import os
import re
import sys
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from typing import Any, Optional

import requests
from git import Repo  # pip install GitPython
from tqdm import tqdm

OWNER = "rerun-io"
REPO = "egui_crates"
INCLUDE_LABELS = False  # It adds quite a bit of visual noise

REPO_ROOT = Path(__file__).resolve().parent.parent

# The published crates. Each lives in `crates/<name>`, and has a PR label with the same name.
CRATES = ["egui_tiles", "egui_table", "egui_mcp"]


@dataclass
class PrInfo:
    gh_user_name: str
    pr_title: str
    labels: list[str]


@dataclass
class CommitInfo:
    hexsha: str
    title: str
    pr_number: Optional[int]


def get_github_token() -> str:
    token = os.environ.get("GH_ACCESS_TOKEN", "")
    if token != "":
        return token

    home_dir = os.path.expanduser("~")
    token_file = os.path.join(home_dir, ".githubtoken")

    try:
        with open(token_file, encoding="utf8") as f:
            token = f.read().strip()
        return token
    except Exception:
        pass

    print("ERROR: expected a GitHub token in the environment variable GH_ACCESS_TOKEN or in ~/.githubtoken")
    sys.exit(1)


# Slow
def fetch_pr_info_from_commit_info(commit_info: CommitInfo) -> Optional[PrInfo]:
    if commit_info.pr_number is None:
        return None
    else:
        return fetch_pr_info(commit_info.pr_number)


# Slow
def fetch_pr_info(pr_number: int) -> Optional[PrInfo]:
    url = f"https://api.github.com/repos/{OWNER}/{REPO}/pulls/{pr_number}"
    gh_access_token = get_github_token()
    headers = {"Authorization": f"Token {gh_access_token}"}
    response = requests.get(url, headers=headers)
    json = response.json()

    # Check if the request was successful (status code 200)
    if response.status_code == 200:
        labels = [label["name"] for label in json["labels"]]
        gh_user_name = json["user"]["login"]
        return PrInfo(gh_user_name=gh_user_name, pr_title=json["title"], labels=labels)
    else:
        print(f"ERROR {url}: {response.status_code} - {json['message']}")
        return None


def get_commit_info(commit: Any) -> CommitInfo:
    # Squash-merge commits:
    if match := re.match(r"(.*) \(#(\d+)\)", commit.summary):
        title = str(match.group(1))
        pr_number = int(match.group(2))
        return CommitInfo(hexsha=commit.hexsha, title=title, pr_number=pr_number)

    # Normal merge commits:
    elif match := re.match(r"Merge pull request #(\d+) from (.*)", commit.summary):
        title = str(match.group(2))
        pr_number = int(match.group(1))
        return CommitInfo(hexsha=commit.hexsha, title=title, pr_number=pr_number)

    else:
        return CommitInfo(hexsha=commit.hexsha, title=commit.summary, pr_number=None)


def print_section(crate: str, items: list[str]) -> None:
    if 0 < len(items):
        print(f"#### {crate}")
        for line in items:
            print(f"* {line}")
    print()


def current_version(crate: str) -> str:
    """The version in Cargo.toml, which is the last release until we bump it."""
    cargo_toml = (REPO_ROOT / "crates" / crate / "Cargo.toml").read_text(encoding="utf8")
    match = re.search(r'^version = "(.+)"', cargo_toml, flags=re.MULTILINE)
    assert match, f"No version in crates/{crate}/Cargo.toml"
    return match.group(1)


def touches(commit: Any, crate: str) -> bool:
    return any(file.startswith(f"crates/{crate}/") for file in commit.stats.files)


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate the changelog of one crate.")
    parser.add_argument("--crate", required=True, choices=CRATES, help="The crate to release")
    parser.add_argument("--version", required=True, help="The version of the new release, e.g. 0.42.0")
    parser.add_argument("--since", help="The tag or commit of the last release. Default: CRATE-vX.Y.Z of Cargo.toml")
    args = parser.parse_args()

    since = args.since or f"{args.crate}-v{current_version(args.crate)}"

    repo = Repo(REPO_ROOT)
    commits = list(repo.iter_commits(f"{since}..HEAD", first_parent=True))
    commits.reverse()  # Most recent last
    commit_infos = list(map(get_commit_info, commits))

    pool = multiprocessing.Pool()
    pr_infos = list(
        tqdm(
            pool.imap(fetch_pr_info_from_commit_info, commit_infos),
            total=len(commit_infos),
            desc="Fetch PR info commits",
        )
    )

    changelog = (REPO_ROOT / "crates" / args.crate / "CHANGELOG.md").read_text(encoding="utf8")

    prs = []
    unsorted_prs = []
    unsorted_commits = []

    for commit, commit_info, pr_info in zip(commits, commit_infos, pr_infos):
        hexsha = commit_info.hexsha
        title = commit_info.title
        title = title.rstrip(".").strip()  # Some PR end with an unnecessary period
        pr_number = commit_info.pr_number
        labels = pr_info.labels if pr_info else []

        belongs = args.crate in labels or touches(commit, args.crate)
        no_crate = not any(crate in labels or touches(commit, crate) for crate in CRATES)

        if pr_number is None:
            # Someone committed straight to main:
            if belongs:
                summary = f"{title} [{hexsha[:7]}](https://github.com/{OWNER}/{REPO}/commit/{hexsha})"
                unsorted_commits.append(summary)
        elif belongs or no_crate:
            pr_link = f"https://github.com/{OWNER}/{REPO}/pull/{pr_number}"
            if f"({pr_link})" in changelog:
                continue  # Already released, e.g. in a patch release

            # We prefer the PR title if available
            title = pr_info.pr_title if pr_info else title

            if "exclude from changelog" in labels:
                continue
            if "typo" in labels:
                # We get so many typo PRs. Let's not flood the changelog with them.
                continue

            summary = f"{title} [#{pr_number}]({pr_link})"

            if INCLUDE_LABELS and 0 < len(labels):
                summary += f" ({', '.join(labels)})"

            if pr_info is not None:
                gh_user_name = pr_info.gh_user_name
                summary += f" by [@{gh_user_name}](https://github.com/{gh_user_name})"

            summary = summary[0].upper() + summary[1:]  # Upper-case first letter

            if belongs:
                prs.append(summary)
            else:
                unsorted_prs.append(summary)

    print(f"## {args.version} - {date.today()}")
    print()
    print(f"Full diff at https://github.com/{OWNER}/{REPO}/compare/{since}..{args.crate}-v{args.version}")
    print()
    for line in prs:
        print(f"* {line}")
    print()
    print_section("PRs without a crate label that change no crate (check by hand)", unsorted_prs)
    print_section("Unsorted commits", unsorted_commits)


if __name__ == "__main__":
    main()
