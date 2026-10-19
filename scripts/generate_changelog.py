#!/usr/bin/env python3
# Based on https://github.com/rerun-io/rerun_template, changed for this monorepo:
# all published crates share one version, and we sort each PR into crates by its labels, like egui.

"""
Summarizes the PRs since the last release, one section per published crate.

A PR goes into the changelog of every crate it has a label for.
CI requires each PR to have a crate label or `exclude from changelog`.

With `--write`, the sections go into each `crates/CRATE/CHANGELOG.md`.
PRs already in a changelog (e.g. from a patch release) are skipped.
You will often want to edit the result by hand.
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

# The published crates. Each lives in `crates/<name>`, and has a PR label with the same name.
# Keep in sync with `.github/workflows/labels.yml`.
CRATES = ["egui_tiles", "egui_table", "egui_mcp"]

REPO_ROOT = Path(__file__).resolve().parent.parent


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


def calc_since_version(new_version: str) -> str:
    parts = new_version.split(".")
    assert len(parts) == 3, "Expected version to be on the format X.Y.Z"
    major = int(parts[0])
    minor = int(parts[1])
    patch = int(parts[2])

    if 0 < patch:
        # A patch release.
        # Include changes since last patch release.
        # This assumes we've cherry-picked stuff for this release.
        return f"{major}.{minor}.{patch - 1}"
    elif 0 < minor:
        # A minor release.
        # The diff spans everything since the last minor release.
        # PRs from intervening patch releases are already in the changelogs, so we skip them.
        return f"{major}.{minor - 1}.0"
    else:
        # A major release.
        # The diff spans everything since the last major release.
        # PRs from intervening minor/patch releases are already in the changelogs, so we skip them.
        return f"{major - 1}.{minor}.0"


def changelog_path(crate: str) -> Path:
    return REPO_ROOT / "crates" / crate / "CHANGELOG.md"


def pr_link(pr_number: int) -> str:
    return f"https://github.com/{OWNER}/{REPO}/pull/{pr_number}"


def changelog_section(version: str, since: str, items: list[str]) -> str:
    result = f"## {version} - {date.today()}\n"
    result += f"Full diff at https://github.com/{OWNER}/{REPO}/compare/{since}..{version}\n\n"
    if items:
        for line in items:
            result += f"* {line}\n"
    else:
        result += "Nothing new\n"
    return result


def add_to_changelog_file(crate: str, section: str) -> None:
    """Insert the section above the newest version in the changelog."""
    path = changelog_path(crate)
    content = path.read_text(encoding="utf8")
    match = re.search(r"^## ", content, flags=re.MULTILINE)
    position = match.start() if match else len(content)
    content = content[:position] + section + "\n\n" + content[position:]
    path.write_text(content, encoding="utf8")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate the changelogs of all published crates.")
    parser.add_argument("--version", required=True, help="The version of the new release, e.g. 0.42.0")
    parser.add_argument("--since", help="The tag or commit to diff from. Default: calculated from --version")
    parser.add_argument("--write", help="Write into the CHANGELOG.md of each crate", action="store_true")
    args = parser.parse_args()

    since = args.since or calc_since_version(args.version)

    repo = Repo(REPO_ROOT)
    # `--first-parent` skips the history of the repos we imported.
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

    # PRs from an earlier patch release are already in a changelog, so we skip them.
    all_changelogs = "".join(changelog_path(crate).read_text(encoding="utf8") for crate in CRATES)

    crate_items: dict[str, list[str]] = {crate: [] for crate in CRATES}
    unsorted_prs = []
    unsorted_commits = []

    for commit_info, pr_info in zip(commit_infos, pr_infos):
        hexsha = commit_info.hexsha
        title = commit_info.title
        title = title.rstrip(".").strip()  # Some PR end with an unnecessary period
        pr_number = commit_info.pr_number

        if pr_number is None:
            # Someone committed straight to main. We never write these into a changelog.
            summary = f"{title} [{hexsha[:7]}](https://github.com/{OWNER}/{REPO}/commit/{hexsha})"
            unsorted_commits.append(summary)
            continue

        if f"({pr_link(pr_number)})" in all_changelogs:
            continue  # Already released, e.g. in a patch release

        # We prefer the PR title if available
        title = pr_info.pr_title if pr_info else title
        labels = pr_info.labels if pr_info else []

        if "exclude from changelog" in labels:
            continue
        if "typo" in labels:
            # We get so many typo PRs. Let's not flood the changelog with them.
            continue

        summary = f"{title} [#{pr_number}]({pr_link(pr_number)})"

        if INCLUDE_LABELS and 0 < len(labels):
            summary += f" ({', '.join(labels)})"

        if pr_info is not None:
            gh_user_name = pr_info.gh_user_name
            summary += f" by [@{gh_user_name}](https://github.com/{gh_user_name})"

        summary = summary[0].upper() + summary[1:]  # Upper-case first letter

        crates = [crate for crate in CRATES if crate in labels]
        if not crates:
            unsorted_prs.append(summary)
        for crate in crates:
            crate_items[crate].append(summary)

    for crate in CRATES:
        print(f"# {crate}")
        print(changelog_section(args.version, since, crate_items[crate]))

    if unsorted_prs:
        print("# PRs without a crate label (not written)")
        for line in unsorted_prs:
            print(f"* {line}")
        print()

    if unsorted_commits:
        print("# Commits without a PR (not written)")
        for line in unsorted_commits:
            print(f"* {line}")
        print()

    if args.write:
        for crate in CRATES:
            add_to_changelog_file(crate, changelog_section(args.version, since, crate_items[crate]))
        print("Wrote the changelogs. Check them by hand before you commit.")


if __name__ == "__main__":
    main()
