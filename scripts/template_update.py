#!/usr/bin/env python3
# Copied from https://github.com/rerun-io/rerun_template

"""
The script has two purposes.

After using `rerun_template` as a template, run this to clean out things you don't need.
Use `scripts/template_update.py init --languages cpp,rust,python` for this.

Update an existing repository with the latest changes from the template.
Use `scripts/template_update.py update --languages cpp,rust,python` for this.

In either case, make sure the list of languages matches the languages you want to support.
You can also use `--dry-run` to see what would happen without actually changing anything.
"""

from __future__ import annotations

import argparse
import os
import shutil
import tempfile

from git import Repo  # pip install GitPython

OWNER = "rerun-io"

# Don't overwrite these when updating existing repository from the template
DO_NOT_OVERWRITE = {
    "Cargo.lock",
    "CHANGELOG.md",
    "main.py",
    "new_project_name/__init__.py",
    "pixi.lock",
    "README.md",
    "requirements.txt",
}

# Files required by C++, but not by _both_ Python and Rust
CPP_FILES = {
    ".clang-format",
    ".github/workflows/cpp.yml",
    "CMakeLists.txt",
    "pixi.lock",  # Pixi is only C++ & Python - For Rust we only use cargo
    "pixi.toml",  # Pixi is only C++ & Python - For Rust we only use cargo
    "src/",
    "src/main.cpp",
}

# Files required by Python, but not by _both_ C++ and Rust
PYTHON_FILES = {
    ".github/workflows/python.yml",
    ".mypy.ini",
    "main.py",
    "new_project_name/__init__.py",
    "pixi.lock",  # Pixi is only C++ & Python - For Rust we only use cargo
    "pixi.toml",  # Pixi is only C++ & Python - For Rust we only use cargo
    "pyproject.toml",
    "requirements.txt",
}

# Files required by Rust, but not by _both_ C++ and Python
RUST_FILES = {
    ".github/workflows/cargo_shear.yml",
    ".github/workflows/rust.yml",
    "bacon.toml",
    "Cargo.lock",
    "Cargo.toml",
    "CHANGELOG.md",  # We only keep a changelog for Rust crates at the moment
    "clippy.toml",
    "Cranky.toml",
    "crates/new_project_name/Cargo.toml",
    "crates/new_project_name/src/",
    "crates/new_project_name/src/lib.rs",
    "crates/new_project_name/src/main.rs",
    "deny.toml",
    "RELEASES.md",
    "rust-toolchain",
    "scripts/clippy_wasm/",
    "scripts/clippy_wasm/clippy.toml",
    "scripts/generate_changelog.py",  # We only keep a changelog for Rust crates at the moment
}

# Files we used to have, but have been removed in never version of rerun_template
DEAD_FILES = ["bacon.toml", "Cranky.toml"]


def parse_languages(lang_str: str) -> set[str]:
    languages = lang_str.split(",") if lang_str else []
    for lang in languages:
        assert lang in ["cpp", "python", "rust"], f"Unsupported language: {lang}"
    return set(languages)


def calc_deny_set(languages: set[str]) -> set[str]:
    """The set of files to delete/ignore."""
    files_to_delete = CPP_FILES | PYTHON_FILES | RUST_FILES
    if "cpp" in languages:
        files_to_delete -= CPP_FILES
    if "python" in languages:
        files_to_delete -= PYTHON_FILES
    if "rust" in languages:
        files_to_delete -= RUST_FILES
    return files_to_delete


def init(languages: set[str], dry_run: bool) -> None:
    print("Removing all language-specific files not needed for languages {languages}.")
    files_to_delete = calc_deny_set(languages)
    delete_files_and_folder(files_to_delete, dry_run)


def remove_file(filepath: str) -> None:
    try:
        os.remove(filepath)
    except FileNotFoundError:
        pass


def delete_files_and_folder(paths: set[str], dry_run: bool) -> None:
    repo_path = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
    for path in paths:
        full_path = os.path.join(repo_path, path)
        if os.path.exists(full_path):
            if os.path.isfile(full_path):
                print(f"Removing file {full_path}…")
                if not dry_run:
                    remove_file(full_path)
            elif os.path.isdir(full_path):
                print(f"Removing folder {full_path}…")
                if not dry_run:
                    shutil.rmtree(full_path)


def update(languages: set[str], dry_run: bool) -> None:
    for file in DEAD_FILES:
        print(f"Removing dead file {file}…")
        if not dry_run:
            remove_file(file)

    files_to_ignore = calc_deny_set(languages) | DO_NOT_OVERWRITE
    repo_path = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))

    with tempfile.TemporaryDirectory() as temp_dir:
        Repo.clone_from("https://github.com/rerun-io/rerun_template.git", temp_dir)
        for root, dirs, files in os.walk(temp_dir):
            for file in files:
                src_path = os.path.join(root, file)
                rel_path = os.path.relpath(src_path, temp_dir)

                if rel_path.startswith(".git/"):
                    continue
                if rel_path.startswith("crates/") or rel_path.startswith("src/"):
                    continue
                if rel_path in files_to_ignore:
                    continue

                dest_path = os.path.join(repo_path, rel_path)

                print(f"Updating {rel_path}…")
                if not dry_run:
                    os.makedirs(os.path.dirname(dest_path), exist_ok=True)
                    shutil.copy2(src_path, dest_path)


def main() -> None:
    parser = argparse.ArgumentParser(description="Handle the Rerun template.")
    subparsers = parser.add_subparsers(dest="command")

    init_parser = subparsers.add_parser("init", help="Initialize a new checkout of the template.")
    init_parser.add_argument(
        "--languages", default="", nargs="?", const="", help="The languages to support (e.g. `cpp,python,rust`)."
    )
    init_parser.add_argument("--dry-run", action="store_true", help="Don't actually delete any files.")

    update_parser = subparsers.add_parser(
        "update", help="Update all existing Rerun repositories with the latest changes from the template"
    )
    update_parser.add_argument(
        "--languages", default="", nargs="?", const="", help="The languages to support (e.g. `cpp,python,rust`)."
    )
    update_parser.add_argument("--dry-run", action="store_true", help="Don't actually delete any files.")

    args = parser.parse_args()

    if args.command == "init":
        init(parse_languages(args.languages), args.dry_run)
    elif args.command == "update":
        update(parse_languages(args.languages), args.dry_run)
    else:
        parser.print_help()
        exit(1)


if __name__ == "__main__":
    main()
