#!/usr/bin/env sh
# The quality gate of the BeautifulRust skill (rust-best-practices §2): format,
# clippy with warnings as errors, tests. Used by CI, the git pre-commit hook and
# the agents' Stop hook (.claude/settings.json). Exit 2 tells Claude Code to keep working.
set -u
cd "$(dirname "$0")/.." || exit 2
# Twelve rustc jobs linking test binaries at once ran Windows out of memory (os error 1453/1455,
# STATUS_STACK_BUFFER_OVERRUN) and left invalid metadata behind (E0786); a caller can still set more.
: "${CARGO_BUILD_JOBS:=4}"
export CARGO_BUILD_JOBS

fail() {
    echo "quality gate: $1 — fix it before handing back (see CLAUDE.md)" >&2
    exit 2
}

cargo fmt --all --check >/dev/null 2>&1 || fail "cargo fmt --check failed (run cargo fmt)"
if ! clippy=$(cargo clippy --workspace --all-targets --quiet -- -D warnings 2>&1); then
    printf '%s\n' "$clippy" | tail -n 40 >&2
    fail "clippy reported errors"
fi
# --no-fail-fast runs every test binary, so one flaky screen test does not hide the others; on
# failure the panics and the list of failed tests are printed (CI logs show nothing else).
if ! tests=$(cargo test --workspace --no-fail-fast 2>&1); then
    printf '%s\n' "$tests" | grep -E -A3 "panicked at|^failures:|FAILED|^error" | tail -n 80 >&2
    fail "cargo test failed (run cargo test --workspace)"
fi
echo "quality gate: ok"
