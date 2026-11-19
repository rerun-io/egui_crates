#!/usr/bin/env bash
# This scripts runs various CI-like checks in a convenient way.

set -eu
script_path=$( cd "$(dirname "${BASH_SOURCE[0]}")" ; pwd -P )
cd "$script_path"
set -x

export RUSTFLAGS="--deny warnings"
export RUSTDOCFLAGS="--deny warnings"

cargo fmt --all -- --check
cargo clippy --quiet --workspace --all-targets --all-features -- --deny warnings
cargo test --quiet --workspace --all-targets --all-features
cargo test --quiet --workspace --doc --all-features # checks all doc-tests

cargo doc --quiet --workspace --no-deps --all-features
cargo doc --quiet --workspace --document-private-items --no-deps --all-features

cargo deny --all-features --log-level error check
cargo shear # cargo install cargo-shear

typos # cargo install typos-cli

echo "All checks passed!"
