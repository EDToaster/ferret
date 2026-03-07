//! End-to-end integration tests for the core indexing and search pipeline.
//!
//! These tests exercise: index files -> search -> incremental updates,
//! using fixture files under `tests/fixtures/repo/`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use ferret_indexer_core::{
    ChangeEvent, ChangeKind, InputFile, Language, SearchOptions, SegmentManager, parse_query,
    search_segments, search_segments_with_options, search_segments_with_query,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the path to the `tests/fixtures/repo/` directory.
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("repo")
}

/// Relative paths (from the fixture repo root) of every fixture file.
const FIXTURE_PATHS: &[&str] = &[
    "src/main.rs",
    "src/lib.rs",
    "src/utils.py",
    "README.md",
    "data/config.toml",
];

/// Load all fixture files as `InputFile`s, reading content from disk.
fn load_fixture_files() -> Vec<InputFile> {
    let base = fixtures_dir();
    FIXTURE_PATHS
        .iter()
        .map(|rel| {
            let full = base.join(rel);
            let content = fs::read(&full)
                .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", full.display(), e));
            InputFile {
                path: rel.to_string(),
                content,
                mtime: 1_700_000_000,
            }
        })
        .collect()
}

/// Create a `SegmentManager`, index all fixture files, and return the manager.
fn build_index(index_dir: &Path) -> SegmentManager {
    let mgr = SegmentManager::new(index_dir).expect("SegmentManager::new");
    let files = load_fixture_files();
    mgr.index_files(files).expect("index_files");
    mgr
}

/// Copy all fixture files into `target`, preserving the relative directory
/// structure. Returns the list of relative paths copied.
fn copy_fixtures(target: &Path) -> Vec<String> {
    let base = fixtures_dir();
    for rel in FIXTURE_PATHS {
        let src = base.join(rel);
        let dst = target.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::copy(&src, &dst)
            .unwrap_or_else(|e| panic!("copy {} -> {}: {}", src.display(), dst.display(), e));
    }
    FIXTURE_PATHS.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Task 2: Index + basic search tests
// ---------------------------------------------------------------------------

#[test]
fn test_index_known_files() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = build_index(tmp.path());

    let snapshot = mgr.snapshot();
    // Should produce exactly 1 segment.
    assert_eq!(snapshot.len(), 1, "expected 1 segment");

    // The segment should contain all 5 fixture files.
    let seg = &snapshot[0];
    assert_eq!(seg.entry_count(), 5, "expected 5 files in segment");

    // Verify all expected paths are present in metadata.
    let reader = seg.metadata_reader();
    let indexed_paths: HashSet<String> = reader
        .iter_all()
        .map(|r| r.expect("read metadata").path.clone())
        .collect();

    for expected in FIXTURE_PATHS {
        assert!(
            indexed_paths.contains(*expected),
            "expected path '{}' not found in index; got: {:?}",
            expected,
            indexed_paths
        );
    }
}

#[test]
fn test_search_literal() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = build_index(tmp.path());
    let snapshot = mgr.snapshot();

    let result = search_segments(&snapshot, "fn main").expect("search");
    assert!(
        !result.files.is_empty(),
        "expected at least 1 file match for 'fn main', got 0"
    );

    let main_match = result
        .files
        .iter()
        .find(|f| f.path.as_os_str() == "src/main.rs")
        .expect("src/main.rs should match 'fn main'");

    // "fn main() {" is on line 8 of src/main.rs (1-indexed).
    // Line 1: use std::collections::HashMap;
    // Line 2: (blank)
    // Line 3: struct Config {
    // Line 4:     name: String,
    // Line 5:     values: HashMap<String, i32>,
    // Line 6: }
    // Line 7: (blank)
    // Line 8: fn main() {
    let has_line_8 = main_match.lines.iter().any(|l| l.line_number == 8);
    assert!(
        has_line_8,
        "expected match on line 8 of src/main.rs; got lines: {:?}",
        main_match
            .lines
            .iter()
            .map(|l| l.line_number)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_search_no_results() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = build_index(tmp.path());
    let snapshot = mgr.snapshot();

    let result = search_segments(&snapshot, "zzz_nonexistent_string_xyz").expect("search");
    assert_eq!(
        result.files.len(),
        0,
        "expected 0 results for nonexistent query"
    );
}

// ---------------------------------------------------------------------------
// Task 3: Regex, filters, case-insensitive, context tests
// ---------------------------------------------------------------------------

#[test]
fn test_search_regex() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = build_index(tmp.path());
    let snapshot = mgr.snapshot();

    let query = parse_query("/def \\w+/").expect("parse regex query");
    let options = SearchOptions::default();
    let result = search_segments_with_query(&snapshot, &query, &options).expect("regex search");

    // utils.py has multiple `def ...` lines; .rs files should not match.
    let py_matches: Vec<_> = result
        .files
        .iter()
        .filter(|f| f.path.to_string_lossy().ends_with(".py"))
        .collect();
    assert!(
        !py_matches.is_empty(),
        "expected utils.py to match /def \\w+/"
    );

    let rs_matches: Vec<_> = result
        .files
        .iter()
        .filter(|f| f.path.to_string_lossy().ends_with(".rs"))
        .collect();
    assert!(
        rs_matches.is_empty(),
        "expected no .rs files to match /def \\w+/, got: {:?}",
        rs_matches.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

#[test]
fn test_search_path_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = build_index(tmp.path());
    let snapshot = mgr.snapshot();

    // "fn" appears in both .rs files under src/. path:src/ should exclude
    // README.md and data/config.toml.
    let query = parse_query("path:src/ fn").expect("parse path filter query");
    let options = SearchOptions::default();
    let result =
        search_segments_with_query(&snapshot, &query, &options).expect("path filter search");

    assert!(
        !result.files.is_empty(),
        "expected at least 1 result for 'path:src/ fn'"
    );

    for fm in &result.files {
        let path_str = fm.path.to_string_lossy();
        assert!(
            path_str.starts_with("src/"),
            "expected path to start with 'src/', got '{}'",
            path_str
        );
    }
}

#[test]
fn test_search_language_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = build_index(tmp.path());
    let snapshot = mgr.snapshot();

    let query = parse_query("lang:python def").expect("parse lang filter query");
    let options = SearchOptions::default();
    let result =
        search_segments_with_query(&snapshot, &query, &options).expect("language filter search");

    assert!(
        !result.files.is_empty(),
        "expected at least 1 result for 'lang:python def'"
    );

    for fm in &result.files {
        assert_eq!(
            fm.language,
            Language::Python,
            "expected only Python files, got {:?} for {}",
            fm.language,
            fm.path.display()
        );
    }
}

#[test]
fn test_search_case_insensitive() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = build_index(tmp.path());
    let snapshot = mgr.snapshot();

    // Default search is case-insensitive. "FN MAIN" should match "fn main".
    let result = search_segments(&snapshot, "FN MAIN").expect("case-insensitive search");
    assert!(
        !result.files.is_empty(),
        "expected case-insensitive match for 'FN MAIN'"
    );

    let main_match = result
        .files
        .iter()
        .find(|f| f.path.as_os_str() == "src/main.rs");
    assert!(
        main_match.is_some(),
        "expected src/main.rs to match 'FN MAIN' case-insensitively"
    );
}

#[test]
fn test_search_context_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = build_index(tmp.path());
    let snapshot = mgr.snapshot();

    let options = SearchOptions {
        context_lines: 2,
        max_results: None,
    };
    let result =
        search_segments_with_options(&snapshot, "fn main", &options).expect("search with context");

    let main_match = result
        .files
        .iter()
        .find(|f| f.path.as_os_str() == "src/main.rs")
        .expect("src/main.rs should match");

    // The "fn main" match on line 8 should have context lines.
    let line_match = main_match
        .lines
        .iter()
        .find(|l| l.line_number == 8)
        .expect("expected match on line 8");

    // With context_lines=2, we expect up to 2 lines before and 2 lines after.
    // Line 8 has lines 6-7 before (line 6: "}", line 7: blank) and 9-10 after.
    assert!(
        !line_match.context_before.is_empty(),
        "expected context_before to be populated; got empty"
    );
    assert!(
        !line_match.context_after.is_empty(),
        "expected context_after to be populated; got empty"
    );
}

// ---------------------------------------------------------------------------
// Task 4: Incremental modify and delete tests
// ---------------------------------------------------------------------------

#[test]
fn test_incremental_modify() {
    let tmp = tempfile::tempdir().unwrap();
    let index_dir = tmp.path().join("index");
    let work_dir = tmp.path().join("repo");
    fs::create_dir_all(&work_dir).unwrap();

    // Copy fixtures into work_dir and build index from them.
    copy_fixtures(&work_dir);

    let mgr = SegmentManager::new(&index_dir).expect("SegmentManager::new");
    let files: Vec<InputFile> = FIXTURE_PATHS
        .iter()
        .map(|rel| {
            let full = work_dir.join(rel);
            let content = fs::read(&full).unwrap();
            InputFile {
                path: rel.to_string(),
                content,
                mtime: 1_700_000_000,
            }
        })
        .collect();
    mgr.index_files(files).expect("initial index_files");

    // Verify the unique marker is NOT yet searchable.
    let snapshot = mgr.snapshot();
    let pre = search_segments(&snapshot, "UNIQUE_MARKER_STRING").expect("pre-search");
    assert_eq!(
        pre.files.len(),
        0,
        "marker should not exist before modification"
    );

    // Modify src/main.rs: append a line containing the marker.
    let main_path = work_dir.join("src/main.rs");
    let mut content = fs::read_to_string(&main_path).unwrap();
    content.push_str("\n// UNIQUE_MARKER_STRING\n");
    fs::write(&main_path, &content).unwrap();

    // Apply the modification.
    let changes = vec![ChangeEvent {
        path: PathBuf::from("src/main.rs"),
        kind: ChangeKind::Modified,
    }];
    mgr.apply_changes(&work_dir, &changes)
        .expect("apply_changes for modify");

    // The marker should now be searchable.
    let snapshot = mgr.snapshot();
    let post = search_segments(&snapshot, "UNIQUE_MARKER_STRING").expect("post-search");
    assert!(
        !post.files.is_empty(),
        "UNIQUE_MARKER_STRING should be found after modification"
    );
    assert!(
        post.files
            .iter()
            .any(|f| f.path.as_os_str() == "src/main.rs"),
        "modified src/main.rs should contain the marker"
    );
}

#[test]
fn test_incremental_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let index_dir = tmp.path().join("index");
    let work_dir = tmp.path().join("repo");
    fs::create_dir_all(&work_dir).unwrap();

    // Copy fixtures and build index.
    copy_fixtures(&work_dir);

    let mgr = SegmentManager::new(&index_dir).expect("SegmentManager::new");
    let files: Vec<InputFile> = FIXTURE_PATHS
        .iter()
        .map(|rel| {
            let full = work_dir.join(rel);
            let content = fs::read(&full).unwrap();
            InputFile {
                path: rel.to_string(),
                content,
                mtime: 1_700_000_000,
            }
        })
        .collect();
    mgr.index_files(files).expect("initial index_files");

    // "DataProcessor" should be found in utils.py before deletion.
    let snapshot = mgr.snapshot();
    let pre = search_segments(&snapshot, "DataProcessor").expect("pre-delete search");
    assert!(
        !pre.files.is_empty(),
        "DataProcessor should be found before deletion"
    );

    // Delete utils.py from the working directory and apply the change.
    fs::remove_file(work_dir.join("src/utils.py")).unwrap();
    let changes = vec![ChangeEvent {
        path: PathBuf::from("src/utils.py"),
        kind: ChangeKind::Deleted,
    }];
    mgr.apply_changes(&work_dir, &changes)
        .expect("apply_changes for delete");

    // "DataProcessor" should no longer be found.
    let snapshot = mgr.snapshot();
    let post = search_segments(&snapshot, "DataProcessor").expect("post-delete search");
    assert_eq!(
        post.files.len(),
        0,
        "DataProcessor should not be found after deleting utils.py"
    );
}
