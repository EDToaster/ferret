# CI Pipeline Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Set up GitHub Actions CI with check, clippy, test, and fmt jobs across macOS and Linux.

**Architecture:** Single workflow file with 4 jobs, matrix strategy for OS, shared caching.

**Tech Stack:** GitHub Actions, dtolnay/rust-toolchain, Swatinem/rust-cache

---

## Task 1: Create `.rustfmt.toml`

**File:** `.rustfmt.toml` (repo root)

Create the file with the following content:

```toml
edition = "2024"
```

**Verification:** File exists at repo root with correct content.

---

## Task 2: Create `.github/workflows/ci.yml`

**File:** `.github/workflows/ci.yml`

Create the directory structure and workflow file with the complete YAML below:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

jobs:
  check:
    name: Check
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --workspace

  clippy:
    name: Clippy
    needs: check
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace -- -D warnings

  test:
    name: Test
    needs: check
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace

  fmt:
    name: Format
    needs: check
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all -- --check
```

**Key design decisions:**
- `check` runs first; `clippy`, `test`, and `fmt` depend on `check` via `needs: check` and run in parallel
- `concurrency` cancels in-progress runs when new commits are pushed to the same branch/PR
- Each job uses the OS matrix for both `ubuntu-latest` and `macos-latest`
- `dtolnay/rust-toolchain@stable` is used (not deprecated `actions-rs`)
- `Swatinem/rust-cache@v2` caches cargo registry and target directory
- Only `clippy` job adds the `clippy` component; only `fmt` job adds `rustfmt`

**Verification:**
- YAML indentation is consistent (2-space)
- All 4 jobs defined: `check`, `clippy`, `test`, `fmt`
- Triggers: `push` to `main`, all `pull_request`
- Matrix: `ubuntu-latest` and `macos-latest`
- Job dependencies: clippy/test/fmt all `needs: check`

---

## Task 3: Validate YAML syntax

Run a YAML linter or parser to confirm the workflow file has no syntax errors.

**Verification:** YAML parses without errors.

---

## Task 4: Commit all changes

Stage and commit:
- `.rustfmt.toml`
- `.github/workflows/ci.yml`
- `docs/plans/2026-02-27-ci-pipeline.md`

**Commit message:** `ci: add GitHub Actions CI with check, clippy, test, and fmt jobs`

**Verification:** `git status` shows clean working tree after commit.
