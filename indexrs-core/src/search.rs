//! Search result types for indexrs query results.
//!
//! These types represent the output of the query engine: matched files,
//! matched lines within those files, and aggregate search result metadata.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::types::{FileId, Language};

/// A single matching line within a file.
///
/// Contains the line content and byte-offset ranges indicating which portions
/// of the line matched the query, used for rendering match highlights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineMatch {
    /// 1-based line number within the file.
    pub line_number: u32,
    /// The full text content of the matching line.
    pub content: String,
    /// Byte-offset ranges `(start, end)` within `content` that matched the query.
    /// Used for highlighting matched portions in the output.
    pub ranges: Vec<(usize, usize)>,
}

/// A file that matched a search query, with its matching lines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMatch {
    /// The indexed file ID.
    pub file_id: FileId,
    /// Path to the file relative to the repository root.
    pub path: PathBuf,
    /// Detected programming language of the file.
    pub language: Language,
    /// Lines within the file that matched the query.
    pub lines: Vec<LineMatch>,
    /// Relevance score in the range [0.0, 1.0], higher is more relevant.
    pub score: f64,
}

/// Aggregate result of a search query.
///
/// Contains the matched files, total match count, and query duration.
/// Implements `Display` for plain-text summary output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Total number of matching lines across all files.
    pub total_count: usize,
    /// Files that matched the query, ordered by relevance score (descending).
    pub files: Vec<FileMatch>,
    /// Wall-clock time taken to execute the query.
    pub duration: Duration,
}

impl fmt::Display for SearchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} results in {} files ({:.1?})",
            self.total_count,
            self.files.len(),
            self.duration,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_result_display() {
        let result = SearchResult {
            total_count: 42,
            files: vec![
                FileMatch {
                    file_id: FileId(1),
                    path: PathBuf::from("src/main.rs"),
                    language: Language::Rust,
                    lines: vec![],
                    score: 0.95,
                },
                FileMatch {
                    file_id: FileId(2),
                    path: PathBuf::from("src/lib.rs"),
                    language: Language::Rust,
                    lines: vec![],
                    score: 0.85,
                },
            ],
            duration: Duration::from_millis(5),
        };
        let display = result.to_string();
        assert!(display.contains("42 results"));
        assert!(display.contains("2 files"));
        assert!(display.contains("5"));
    }

    #[test]
    fn test_search_result_display_empty() {
        let result = SearchResult {
            total_count: 0,
            files: vec![],
            duration: Duration::from_micros(100),
        };
        let display = result.to_string();
        assert!(display.contains("0 results"));
        assert!(display.contains("0 files"));
    }

    #[test]
    fn test_line_match_ranges() {
        let line = LineMatch {
            line_number: 10,
            content: "fn parse_query(input: &str) -> Query".to_string(),
            ranges: vec![(3, 14), (31, 36)],
        };
        assert_eq!(line.line_number, 10);
        assert_eq!(line.ranges.len(), 2);
        assert_eq!(
            &line.content[line.ranges[0].0..line.ranges[0].1],
            "parse_query"
        );
        assert_eq!(&line.content[line.ranges[1].0..line.ranges[1].1], "Query");
    }

    #[test]
    fn test_line_match_empty_ranges() {
        let line = LineMatch {
            line_number: 1,
            content: "use std::io;".to_string(),
            ranges: vec![],
        };
        assert!(line.ranges.is_empty());
    }

    #[test]
    fn test_file_match_construction() {
        let file_match = FileMatch {
            file_id: FileId(42),
            path: PathBuf::from("src/types.rs"),
            language: Language::Rust,
            lines: vec![LineMatch {
                line_number: 5,
                content: "pub struct FileId(u32);".to_string(),
                ranges: vec![(11, 17)],
            }],
            score: 0.92,
        };
        assert_eq!(file_match.file_id, FileId(42));
        assert_eq!(file_match.language, Language::Rust);
        assert_eq!(file_match.lines.len(), 1);
        assert!((file_match.score - 0.92).abs() < f64::EPSILON);
    }
}
