//! CLI and MCP integration tests for the ferret binary.
//!
//! These tests exercise the end-to-end CLI workflow: init an index from
//! fixture files, search via the daemon, and verify output format / exit codes.
//! The MCP test validates JSON-RPC framing over stdio.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Path to the fixture repo used by all tests.
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("ferret-core")
        .join("tests")
        .join("fixtures")
        .join("repo")
}

/// Recursively copy `src_dir` into `dst_dir`.
fn copy_fixtures(src: &Path, dst: &Path) {
    for entry in std::fs::read_dir(src).expect("read fixtures dir") {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let dest = dst.join(entry.file_name());
        if ty.is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
            copy_fixtures(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

/// Return the absolute path to the `ferret` binary built by cargo.
fn ferret_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ferret"))
}

/// Copy fixtures into a fresh tempdir, run `ferret init`, and return the
/// tempdir (kept alive by the caller).
fn setup_repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("create tempdir");
    copy_fixtures(&fixtures_dir(), tmp.path());

    // The CLI requires a .git dir (or .ferret_index) to recognize the repo root.
    std::fs::create_dir(tmp.path().join(".git")).unwrap();

    // Initialize the index.
    let output = Command::new(ferret_bin())
        .args(["--repo", tmp.path().to_str().unwrap(), "init"])
        .output()
        .expect("ferret init");

    assert!(
        output.status.success(),
        "ferret init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the segments directory was created.
    assert!(
        tmp.path().join(".ferret_index").join("segments").exists(),
        "segments dir missing after init"
    );

    tmp
}

// ---------------------------------------------------------------------------
// CLI search tests
// ---------------------------------------------------------------------------

#[test]
fn test_cli_search_output_format() {
    let repo = setup_repo();

    let output = Command::new(ferret_bin())
        .args([
            "--color",
            "never",
            "--repo",
            repo.path().to_str().unwrap(),
            "search",
            "fn main",
        ])
        .output()
        .expect("ferret search");

    assert!(
        output.status.success(),
        "search failed (exit {}): {}",
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "search returned no output");

    // Every non-empty line must match the vimgrep format: path:line:col:content
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(4, ':').collect();
        assert!(
            parts.len() == 4,
            "line does not match path:line:col:content format: {line:?}"
        );

        // parts[1] must be a line number.
        let line_num: usize = parts[1]
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("expected line number, got {:?} in line: {line}", parts[1]));
        assert!(line_num > 0, "line number should be positive");

        // parts[2] must be a column number.
        let col: usize = parts[2].trim().parse().unwrap_or_else(|_| {
            panic!("expected column number, got {:?} in line: {line}", parts[2])
        });
        assert!(col > 0, "column number should be positive");
    }
}

#[test]
fn test_cli_search_exit_codes() {
    let repo = setup_repo();
    let repo_path = repo.path().to_str().unwrap();

    // Exit 0: results found.
    let found = Command::new(ferret_bin())
        .args(["--color", "never", "--repo", repo_path, "search", "fn main"])
        .output()
        .expect("ferret search (found)");
    assert_eq!(
        found.status.code(),
        Some(0),
        "expected exit 0 for matching query, stderr: {}",
        String::from_utf8_lossy(&found.stderr)
    );

    // Exit 1: no results.
    let not_found = Command::new(ferret_bin())
        .args([
            "--color",
            "never",
            "--repo",
            repo_path,
            "search",
            "zzz_no_match_zzz",
        ])
        .output()
        .expect("ferret search (not found)");
    assert_eq!(
        not_found.status.code(),
        Some(1),
        "expected exit 1 for non-matching query, stderr: {}",
        String::from_utf8_lossy(&not_found.stderr)
    );
}

#[test]
fn test_cli_search_no_color() {
    let repo = setup_repo();

    let output = Command::new(ferret_bin())
        .args([
            "--color",
            "never",
            "--repo",
            repo.path().to_str().unwrap(),
            "search",
            "fn main",
        ])
        .output()
        .expect("ferret search");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    // ANSI escape sequences start with ESC (0x1b).
    assert!(
        !stdout.contains('\x1b'),
        "output contains ANSI escape codes with --color=never: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// MCP integration test
// ---------------------------------------------------------------------------

#[cfg(feature = "mcp")]
#[test]
fn test_mcp_search_response() {
    let repo = setup_repo();

    let mut child = Command::new(ferret_bin())
        .args(["--repo", repo.path().to_str().unwrap(), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ferret mcp");

    let stdin = child.stdin.as_mut().expect("open stdin");
    let stdout = child.stdout.take().expect("open stdout");
    let mut reader = BufReader::new(stdout);

    // The rmcp stdio transport uses newline-delimited JSON (one JSON object
    // per line), not Content-Length framing.

    // Helper: send a JSON-RPC message as a single line.
    fn send_message(stdin: &mut impl Write, body: &serde_json::Value) {
        let payload = serde_json::to_string(body).unwrap();
        writeln!(stdin, "{payload}").unwrap();
        stdin.flush().unwrap();
    }

    // Helper: read a single JSON-RPC response line.
    fn read_response(reader: &mut BufReader<impl Read>) -> serde_json::Value {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read response line");
        assert!(n > 0, "unexpected EOF while reading response");
        serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("failed to parse JSON response: {e}\nraw line: {line}"))
    }

    // 1. Send initialize request.
    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "test-client",
                "version": "0.1.0"
            }
        }
    });
    send_message(stdin, &init_request);

    let init_resp = read_response(&mut reader);
    assert!(
        init_resp.get("result").is_some(),
        "initialize response missing result: {init_resp}"
    );

    // 2. Send initialized notification (no id, no response expected).
    let initialized_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    send_message(stdin, &initialized_notif);

    // Small delay to let server process the notification.
    std::thread::sleep(std::time::Duration::from_millis(200));

    // 3. Send tools/call with search_code.
    let search_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "search_code",
            "arguments": {
                "query": "fn main",
                "max_results": 10
            }
        }
    });
    send_message(stdin, &search_request);

    let search_resp = read_response(&mut reader);

    // Verify response structure.
    let result = search_resp
        .get("result")
        .unwrap_or_else(|| panic!("search response missing result: {search_resp}"));

    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("result missing content array: {result}"));

    assert!(!content.is_empty(), "content array is empty");

    // At least one content item should mention main.rs.
    let text_concat: String = content
        .iter()
        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        text_concat.contains("main.rs"),
        "search result text does not mention main.rs: {text_concat}"
    );

    // Clean up.
    drop(child.stdin.take());
    let _ = child.kill();
    let _ = child.wait();
}
