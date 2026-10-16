#!/usr/bin/env python3
# Copied from https://github.com/rerun-io/rerun_template

"""
Summarizes recent PRs based on their GitHub labels.

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
from typing import Any, Optional

import requests
from git import Repo  # pip install GitPython
from tqdm import tqdm

OWNER = "rerun-io"
REPO = "egui_crates"
INCLUDE_LABELS = False  # It adds quite a bit of visual noise


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


def remove_prefix(text: str, prefix: str) -> str:
    if text.startswith(prefix):
        return text[len(prefix) :]
    return text  # or whatever


def print_section(crate: str, items: list[str]) -> None:
    if 0 < len(items):
        print(f"#### {crate}")
        for line in items:
            print(f"* {line}")
    print()


def calc_commit_range(new_version: str) -> str:
    parts = new_version.split(".")
    assert len(parts) == 3, "Expected version to be on the format X.Y.Z"
    major = int(parts[0])
    minor = int(parts[1])
    patch = int(parts[2])

    if 0 < patch:
        # A patch release.
        # Include changes since last patch release.
        # This assumes we've cherry-picked stuff for this release.
        diff_since_version = f"0.{minor}.{patch - 1}"
    elif 0 < minor:
        # A minor release
        # The diff should span everything since the last minor release.
        # The script later excludes duplicated automatically, so we don't include stuff that
        # was part of intervening patch releases.
        diff_since_version = f"{major}.{minor - 1}.0"
    else:
        # A major release
        # The diff should span everything since the last major release.
        # The script later excludes duplicated automatically, so we don't include stuff that
        # was part of intervening minor/patch releases.
        diff_since_version = f"{major - 1}.{minor}.0"

    return f"{diff_since_version}..HEAD"


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate a changelog.")
    parser.add_argument("--version", required=True, help="The version of the new release, e.g. 0.42.0")
    args = parser.parse_args()

    commit_range = calc_commit_range(args.version)

    repo = Repo(".")
    commits = list(repo.iter_commits(commit_range))
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

    prs = []
    unsorted_commits = []

    for commit_info, pr_info in zip(commit_infos, pr_infos):
        hexsha = commit_info.hexsha
        title = commit_info.title
        title = title.rstrip(".").strip()  # Some PR end with an unnecessary period
        pr_number = commit_info.pr_number

        if pr_number is None:
            # Someone committed straight to main:
            summary = f"{title} [{hexsha[:7]}](https://github.com/{OWNER}/{REPO}/commit/{hexsha})"
            unsorted_commits.append(summary)
        else:
            # We prefer the PR title if available
            title = pr_info.pr_title if pr_info else title
            labels = pr_info.labels if pr_info else []

            if "exclude from changelog" in labels:
                continue
            if "typo" in labels:
                # We get so many typo PRs. Let's not flood the changelog with them.
                continue

            summary = f"{title} [#{pr_number}](https://github.com/{OWNER}/{REPO}/pull/{pr_number})"

            if INCLUDE_LABELS and 0 < len(labels):
                summary += f" ({', '.join(labels)})"

            if pr_info is not None:
                gh_user_name = pr_info.gh_user_name
                summary += f" by [@{gh_user_name}](https://github.com/{gh_user_name})"

            prs.append(summary)

    # Clean up:
    for i in range(len(prs)):
        line = prs[i]
        line = line[0].upper() + line[1:]  # Upper-case first letter
        prs[i] = line

    print(f"## {args.version} - {date.today()}")
    print()
    print(f"Full diff at https://github.com/{OWNER}/{REPO}/compare/{commit_range}")
    print()
    print_section("PRs", prs)
    print_section("Unsorted commits", unsorted_commits)


if __name__ == "__main__":
    main()
