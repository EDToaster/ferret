//! CLI and MCP integration tests for the ferret binary.
//!
//! These tests exercise the end-to-end CLI workflow: init an index from
//! fixture files, search via the daemon, and verify output format / exit codes.
//! The MCP test validates JSON-RPC framing over stdio.

use std::io::{BufRead, BufReader, Write};
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
// CLI search: filters, regex, context, case sensitivity, query mode
// ---------------------------------------------------------------------------

#[test]
fn test_cli_search_regex() {
    let repo = setup_repo();

    let output = Command::new(ferret_bin())
        .args([
            "--color",
            "never",
            "--repo",
            repo.path().to_str().unwrap(),
            "search",
            "--regex",
            r"def \w+",
        ])
        .output()
        .expect("ferret search --regex");

    assert!(
        output.status.success(),
        "regex search failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "regex search returned no output");

    // Python file should appear in results.
    assert!(
        stdout.contains("utils.py"),
        "expected utils.py in regex results: {stdout}"
    );

    // Rust files should NOT match `def \w+`.
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        assert!(
            !line.contains(".rs"),
            "unexpected .rs file in regex `def \\w+` results: {line}"
        );
    }
}

#[test]
fn test_cli_search_language_filter() {
    let repo = setup_repo();

    let output = Command::new(ferret_bin())
        .args([
            "--color",
            "never",
            "--repo",
            repo.path().to_str().unwrap(),
            "search",
            "--language",
            "python",
            "def",
        ])
        .output()
        .expect("ferret search --language python");

    assert!(
        output.status.success(),
        "language-filtered search failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.is_empty(),
        "language filter search returned no output"
    );

    // Every result line path must be a .py file.
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let path = line.split(':').next().unwrap();
        assert!(
            path.ends_with(".py"),
            "expected only .py files with --language python, got: {line}"
        );
    }
}

#[test]
fn test_cli_search_path_filter() {
    let repo = setup_repo();

    let output = Command::new(ferret_bin())
        .args([
            "--color",
            "never",
            "--repo",
            repo.path().to_str().unwrap(),
            "search",
            "--path",
            "src/*",
            "fn main",
        ])
        .output()
        .expect("ferret search --path src/*");

    assert!(
        output.status.success(),
        "path-filtered search failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "path filter search returned no output");

    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let path = line.split(':').next().unwrap();
        assert!(
            path.contains("src/"),
            "expected only src/ paths with --path src/*, got: {line}"
        );
        // Should not contain README.md or data/ files.
        assert!(
            !path.contains("README") && !path.contains("data/"),
            "path filter should exclude non-src/ files, got: {line}"
        );
    }
}

#[test]
fn test_cli_search_context_lines() {
    let repo = setup_repo();
    let repo_path = repo.path().to_str().unwrap();

    // Verify that -C flag is accepted and produces valid output.
    // The CLI vimgrep format currently outputs match lines only (context
    // lines are available in the daemon protocol but not rendered in the
    // vimgrep CLI output), so we just verify the flag is accepted and
    // results are still returned correctly.
    let output = Command::new(ferret_bin())
        .args([
            "--color", "never", "--repo", repo_path, "search", "-C", "2", "fn main",
        ])
        .output()
        .expect("ferret search -C 2");

    assert!(
        output.status.success(),
        "search with -C 2 failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "search with -C 2 returned no output");

    // Output should still be valid vimgrep format.
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(4, ':').collect();
        assert_eq!(
            parts.len(),
            4,
            "context search line not in vimgrep format: {line:?}"
        );
    }
}

#[test]
fn test_cli_search_case_sensitive() {
    let repo = setup_repo();
    let repo_path = repo.path().to_str().unwrap();

    // Case-sensitive search for uppercase "FN MAIN" should find nothing
    // because the fixture has lowercase `fn main`.
    let cs = Command::new(ferret_bin())
        .args([
            "--color",
            "never",
            "--repo",
            repo_path,
            "search",
            "--case-sensitive",
            "FN MAIN",
        ])
        .output()
        .expect("ferret search --case-sensitive");
    assert_eq!(
        cs.status.code(),
        Some(1),
        "expected exit 1 for case-sensitive uppercase query, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&cs.stdout),
        String::from_utf8_lossy(&cs.stderr)
    );

    // Case-insensitive search for "FN MAIN" should match.
    let ci = Command::new(ferret_bin())
        .args([
            "--color",
            "never",
            "--repo",
            repo_path,
            "search",
            "--ignore-case",
            "FN MAIN",
        ])
        .output()
        .expect("ferret search --ignore-case");
    assert_eq!(
        ci.status.code(),
        Some(0),
        "expected exit 0 for case-insensitive uppercase query, stderr: {}",
        String::from_utf8_lossy(&ci.stderr)
    );

    let stdout = String::from_utf8_lossy(&ci.stdout);
    assert!(
        stdout.contains("main.rs"),
        "case-insensitive search should find main.rs: {stdout}"
    );
}

#[test]
fn test_cli_search_query_mode() {
    let repo = setup_repo();

    let output = Command::new(ferret_bin())
        .args([
            "--color",
            "never",
            "--repo",
            repo.path().to_str().unwrap(),
            "search",
            "--query",
            "path:src/ lang:rust fn",
        ])
        .output()
        .expect("ferret search --query");

    assert!(
        output.status.success(),
        "query mode search failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "query mode search returned no output");

    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let path = line.split(':').next().unwrap();
        // Paths in CLI output are absolute, so check with contains.
        assert!(
            path.contains("src/") && path.ends_with(".rs"),
            "expected only Rust files under src/ with query mode, got: {line}"
        );
    }
}

// ---------------------------------------------------------------------------
// MCP integration tests
// ---------------------------------------------------------------------------

/// Helper client for MCP integration tests.
///
/// Spawns `ferret mcp`, performs the JSON-RPC initialize handshake, and
/// provides a `call_tool` method for invoking MCP tools.
#[cfg(feature = "mcp")]
struct McpClient {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

#[cfg(feature = "mcp")]
impl McpClient {
    /// Spawn `ferret mcp` against the given repo and complete the initialize
    /// handshake. Panics on any setup failure.
    fn new(repo: &TempDir) -> Self {
        let mut child = Command::new(ferret_bin())
            .args(["--repo", repo.path().to_str().unwrap(), "mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn ferret mcp");

        let stdin = child.stdin.take().expect("open stdin");
        let stdout = child.stdout.take().expect("open stdout");
        let reader = BufReader::new(stdout);

        let mut client = Self {
            child,
            stdin,
            reader,
            next_id: 1,
        };

        // 1. Send initialize request.
        let init_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": client.next_id(),
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
        client.send(&init_request);

        let init_resp = client.read();
        assert!(
            init_resp.get("result").is_some(),
            "initialize response missing result: {init_resp}"
        );

        // 2. Send initialized notification (no id, no response expected).
        let initialized_notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        client.send(&initialized_notif);

        // Small delay to let server process the notification.
        std::thread::sleep(std::time::Duration::from_millis(200));

        client
    }

    /// Allocate the next JSON-RPC request id.
    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Send a JSON-RPC message as a single newline-delimited JSON line.
    fn send(&mut self, body: &serde_json::Value) {
        let payload = serde_json::to_string(body).unwrap();
        writeln!(self.stdin, "{payload}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Read a single JSON-RPC response line.
    fn read(&mut self) -> serde_json::Value {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .expect("read response line");
        assert!(n > 0, "unexpected EOF while reading response");
        serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("failed to parse JSON response: {e}\nraw line: {line}"))
    }

    /// Call an MCP tool by name and return the full JSON-RPC response.
    fn call_tool(&mut self, name: &str, arguments: serde_json::Value) -> serde_json::Value {
        let id = self.next_id();
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            }
        });
        self.send(&request);
        self.read()
    }

    /// Extract the concatenated text from all content items in a tool response.
    fn response_text(resp: &serde_json::Value) -> String {
        let result = resp
            .get("result")
            .unwrap_or_else(|| panic!("response missing result: {resp}"));
        let content = result
            .get("content")
            .and_then(|c| c.as_array())
            .unwrap_or_else(|| panic!("result missing content array: {result}"));
        content
            .iter()
            .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(feature = "mcp")]
impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(feature = "mcp")]
#[test]
fn test_mcp_search_response() {
    let repo = setup_repo();
    let mut client = McpClient::new(&repo);

    let resp = client.call_tool(
        "search_code",
        serde_json::json!({
            "query": "fn main",
            "max_results": 10
        }),
    );

    // Verify response structure.
    let result = resp
        .get("result")
        .unwrap_or_else(|| panic!("search response missing result: {resp}"));

    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("result missing content array: {result}"));

    assert!(!content.is_empty(), "content array is empty");

    let text = McpClient::response_text(&resp);
    assert!(
        text.contains("main.rs"),
        "search result text does not mention main.rs: {text}"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn test_mcp_ping() {
    let repo = setup_repo();
    let mut client = McpClient::new(&repo);

    let resp = client.call_tool("ping", serde_json::json!({}));
    let text = McpClient::response_text(&resp);

    assert!(
        text.to_lowercase().contains("ferret"),
        "ping response should mention ferret: {text}"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn test_mcp_search_files() {
    let repo = setup_repo();
    let mut client = McpClient::new(&repo);

    let resp = client.call_tool("search_files", serde_json::json!({ "query": "main" }));
    let text = McpClient::response_text(&resp);

    assert!(
        text.contains("main.rs"),
        "search_files response should mention main.rs: {text}"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn test_mcp_index_status() {
    let repo = setup_repo();
    let mut client = McpClient::new(&repo);

    let resp = client.call_tool("index_status", serde_json::json!({}));
    let text = McpClient::response_text(&resp);

    assert!(
        text.contains("Segments:"),
        "index_status should contain 'Segments:': {text}"
    );
    assert!(
        text.contains("Files:"),
        "index_status should contain 'Files:': {text}"
    );
    assert!(
        text.contains('5'),
        "index_status should mention 5 files: {text}"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn test_mcp_search_code_error() {
    let repo = setup_repo();
    let mut client = McpClient::new(&repo);

    // Query too short (< 3 chars) — should return a valid JSON-RPC response
    // (either an error or empty results), not crash.
    let resp = client.call_tool("search_code", serde_json::json!({ "query": "ab" }));

    // The response must be valid JSON-RPC (it already parsed successfully in
    // `read`). It should have either a `result` or an `error` field.
    assert!(
        resp.get("result").is_some() || resp.get("error").is_some(),
        "short query response should be a valid JSON-RPC result or error: {resp}"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn test_mcp_search_symbols() {
    let repo = setup_repo();
    let mut client = McpClient::new(&repo);

    let resp = client.call_tool("search_symbols", serde_json::json!({ "query": "main" }));

    // The response should have a result with content.
    let result = resp
        .get("result")
        .unwrap_or_else(|| panic!("search_symbols response missing result: {resp}"));
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("result missing content array: {result}"));
    assert!(!content.is_empty(), "search_symbols content array is empty");

    let text = McpClient::response_text(&resp);
    assert!(
        !text.is_empty(),
        "search_symbols should return non-empty text"
    );
}
