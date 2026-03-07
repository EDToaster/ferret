# End-to-End Integration Tests (HHC-66)

## Goal

Full pipeline integration tests: create fixture files → index → query → verify results. Cover literal search, regex, filters, incremental updates, CLI output format, and MCP responses.

## Test Fixtures

Small deterministic files checked into `ferret-core/tests/fixtures/repo/`:

- `src/main.rs` — Rust: `fn main()`, struct definition, imports
- `src/lib.rs` — Rust: `pub fn search()`, trait, module declarations
- `src/utils.py` — Python: `def helper()`, class, docstring
- `README.md` — Markdown prose
- `data/config.toml` — TOML key-value config

Fixtures are copied to a `tempdir()` per test for isolation.

## ferret-core: `tests/integration.rs`

Library-level tests using `SegmentManager` + `search_segments` directly.

| Test | Verifies |
|------|----------|
| `test_index_known_files` | Segment count, file count, metadata paths match fixtures |
| `test_search_literal` | `"fn main"` → correct file path and line number |
| `test_search_regex` | `/def \w+/` → Python file matches |
| `test_search_path_filter` | `path:src/` excludes README.md and data/ |
| `test_search_language_filter` | `lang:python` returns only .py files |
| `test_search_case_insensitive` | `"FN MAIN"` matches case-insensitively |
| `test_search_context_lines` | `context_lines=2` populates context_before/after |
| `test_incremental_modify` | `apply_changes(Modified)` → search returns updated content |
| `test_incremental_delete` | `apply_changes(Deleted)` → file no longer appears |
| `test_search_no_results` | Nonexistent string returns 0 matches |

## ferret-cli: `tests/cli_integration.rs`

Binary-level tests using `std::process::Command` against the built CLI.

| Test | Verifies |
|------|----------|
| `test_cli_search_output_format` | Lines match `path:line:col:content` vimgrep format |
| `test_cli_search_exit_codes` | Exit 0 for matches, exit 1 for no results |
| `test_cli_search_no_color` | `--color=never` produces parseable plain text |
| `test_mcp_search_response` | MCP JSON-RPC callTool returns valid text with file matches |

CLI tests init a temp repo, index it, then invoke subcommands. MCP tests spawn the process, exchange JSON-RPC messages over stdio.

## Non-goals

- Benchmarking / performance assertions
- Web UI / Playwright tests (covered separately)
- Daemon protocol tests (covered by daemon crate unit tests)
