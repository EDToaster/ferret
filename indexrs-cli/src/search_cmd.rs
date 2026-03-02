use std::sync::Arc;

use globset::{Glob, GlobMatcher};
use indexrs_core::error::IndexError;
use indexrs_core::index_state::SegmentList;
use indexrs_core::multi_search::{
    search_segments_with_pattern_and_options, search_segments_with_query,
};
use indexrs_core::query::{LiteralQuery, Query, RegexQuery, match_language, parse_query};
use indexrs_core::search::{MatchPattern, SearchOptions};

use crate::color::ColorConfig;
use crate::output::{ExitCode, StreamingWriter};
use crate::paths::PathRewriter;

pub struct SearchCmdOptions {
    pub pattern: MatchPattern,
    pub context_lines: usize,
    pub limit: usize,
    pub language: Option<String>,
    pub path_glob: Option<String>,
    #[allow(dead_code)]
    pub stats: bool,
}

/// Resolve CLI flags into a MatchPattern.
///
/// Priority: regex > case_sensitive > ignore_case > smart_case > default (smart case).
/// Smart case: case-sensitive if the query contains any uppercase character,
/// otherwise case-insensitive.
pub fn resolve_match_pattern(
    query: &str,
    regex: bool,
    case_sensitive: bool,
    ignore_case: bool,
    smart_case: bool,
) -> MatchPattern {
    if regex {
        MatchPattern::Regex(query.to_string())
    } else if case_sensitive {
        MatchPattern::Literal(query.to_string())
    } else if ignore_case {
        MatchPattern::LiteralCaseInsensitive(query.to_string())
    } else if smart_case || (!case_sensitive && !ignore_case) {
        // Smart case: if query has uppercase, treat as case-sensitive
        if query.chars().any(|c| c.is_uppercase()) {
            MatchPattern::Literal(query.to_string())
        } else {
            MatchPattern::LiteralCaseInsensitive(query.to_string())
        }
    } else {
        MatchPattern::LiteralCaseInsensitive(query.to_string())
    }
}

/// Convert CLI flags (pattern + optional language) into a Query AST.
///
/// Maps MatchPattern variants to Query leaf nodes. If a language filter is
/// provided, wraps the content query in an AND with LanguageFilter.
///
/// Path glob is NOT included here — Query::PathFilter is prefix-based,
/// but --path supports globs. Path glob filtering stays post-hoc in CLI.
pub fn flags_to_query(pattern: &MatchPattern, language: Option<&str>) -> Result<Query, IndexError> {
    let content_query = match pattern {
        MatchPattern::Literal(s) => Query::Literal(LiteralQuery {
            text: s.clone(),
            case_sensitive: true,
        }),
        MatchPattern::LiteralCaseInsensitive(s) => Query::Literal(LiteralQuery {
            text: s.clone(),
            case_sensitive: false,
        }),
        MatchPattern::Regex(s) => Query::Regex(RegexQuery {
            pattern: s.clone(),
            case_sensitive: true,
        }),
    };

    let Some(lang_str) = language else {
        return Ok(content_query);
    };

    let lang = match_language(lang_str)?;
    Ok(Query::And(vec![Query::LanguageFilter(lang), content_query]))
}

/// Run the search command: search segments, format as vimgrep, stream to output.
///
/// This is the batch version that collects all results before outputting.
/// For incremental output, use [`run_search_streaming`] instead.
#[allow(dead_code)]
pub fn run_search<W: std::io::Write>(
    snapshot: &SegmentList,
    opts: &SearchCmdOptions,
    color: &ColorConfig,
    path_rewriter: &PathRewriter,
    writer: &mut StreamingWriter<W>,
) -> Result<ExitCode, IndexError> {
    let search_opts = SearchOptions {
        context_lines: opts.context_lines,
        max_results: Some(opts.limit),
    };

    let result = search_segments_with_pattern_and_options(snapshot, &opts.pattern, &search_opts)?;

    if result.files.is_empty() {
        return Ok(ExitCode::NoResults);
    }

    let glob_matcher: Option<GlobMatcher> = opts
        .path_glob
        .as_ref()
        .map(|g| Glob::new(g).map(|g| g.compile_matcher()))
        .transpose()
        .map_err(|e| IndexError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?;

    for file_match in &result.files {
        let raw_path = file_match.path.to_string_lossy();

        // Path filter (use raw repo-relative path for glob matching)
        if let Some(ref matcher) = glob_matcher
            && !matcher.is_match(raw_path.as_ref())
        {
            continue;
        }

        // Language filter
        if let Some(ref lang) = opts.language
            && !file_match.language.to_string().eq_ignore_ascii_case(lang)
        {
            continue;
        }

        let path_str = path_rewriter.rewrite(&raw_path);

        for line_match in &file_match.lines {
            let col = line_match
                .ranges
                .first()
                .map(|(start, _)| start + 1)
                .unwrap_or(1);

            let line = color.format_search_line(
                &path_str,
                line_match.line_number,
                col,
                &line_match.content,
                &line_match.ranges,
            );

            if writer.write_line(&line).is_err() {
                break;
            }
        }
    }
    let _ = writer.finish();

    if opts.stats {
        eprintln!(
            "{} matches in {} files ({:.1?})",
            result.total_match_count, result.total_file_count, result.duration
        );
    }

    Ok(ExitCode::Success)
}

/// Run the search command in streaming mode: results are displayed as they're found.
///
/// Uses `search_segments_streaming` to send results through a channel,
/// formatting and writing each one as it arrives. This gives the user
/// immediate feedback, which is critical for fzf integration.
#[allow(dead_code)]
pub fn run_search_streaming<W: std::io::Write>(
    snapshot: &SegmentList,
    opts: &SearchCmdOptions,
    color: &ColorConfig,
    path_rewriter: &PathRewriter,
    writer: &mut StreamingWriter<W>,
) -> Result<ExitCode, IndexError> {
    let search_opts = SearchOptions {
        context_lines: opts.context_lines,
        max_results: Some(opts.limit),
    };

    let glob_matcher: Option<GlobMatcher> = opts
        .path_glob
        .as_ref()
        .map(|g| Glob::new(g).map(|g| g.compile_matcher()))
        .transpose()
        .map_err(|e| IndexError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?;

    let (tx, rx) = std::sync::mpsc::channel();
    let query = flags_to_query(&opts.pattern, opts.language.as_deref())?;

    // Run the search on a background thread so we can consume results on this thread
    let snapshot_clone = Arc::clone(snapshot);
    let search_handle = std::thread::spawn(move || {
        indexrs_core::multi_search::search_segments_with_query_streaming(
            &snapshot_clone,
            &query,
            &search_opts,
            tx,
        )
    });

    let mut has_results = false;
    for file_match in rx {
        let raw_path = file_match.path.to_string_lossy();

        // Path filter (use raw repo-relative path for glob matching)
        if let Some(ref matcher) = glob_matcher
            && !matcher.is_match(raw_path.as_ref())
        {
            continue;
        }

        has_results = true;
        let path_str = path_rewriter.rewrite(&raw_path);

        for line_match in &file_match.lines {
            let col = line_match
                .ranges
                .first()
                .map(|(start, _)| start + 1)
                .unwrap_or(1);

            let line = color.format_search_line(
                &path_str,
                line_match.line_number,
                col,
                &line_match.content,
                &line_match.ranges,
            );

            if writer.write_line(&line).is_err() {
                // SIGPIPE or broken pipe -- stop consuming to cancel search
                break;
            }
        }
    }
    let _ = writer.finish();

    // Check for search errors
    match search_handle.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err(IndexError::Io(std::io::Error::other(
                "search thread panicked",
            )));
        }
    }

    Ok(if has_results {
        ExitCode::Success
    } else {
        ExitCode::NoResults
    })
}

/// Run a search using the advanced query language.
///
/// Parses the query string into a Query AST, executes it through the full
/// query engine pipeline (trigram extraction -> candidate filtering ->
/// boolean verification), and outputs results in vimgrep format.
pub fn run_query_search<W: std::io::Write>(
    snapshot: &SegmentList,
    query_str: &str,
    context_lines: usize,
    limit: usize,
    color: &ColorConfig,
    path_rewriter: &PathRewriter,
    writer: &mut StreamingWriter<W>,
) -> Result<ExitCode, IndexError> {
    let query = parse_query(query_str)?;
    let opts = SearchOptions {
        context_lines,
        max_results: Some(limit),
    };
    let result = search_segments_with_query(snapshot, &query, &opts)?;

    if result.files.is_empty() {
        return Ok(ExitCode::NoResults);
    }

    for file_match in &result.files {
        let raw_path = file_match.path.to_string_lossy();
        let path_str = path_rewriter.rewrite(&raw_path);

        for line_match in &file_match.lines {
            let col = line_match
                .ranges
                .first()
                .map(|(start, _)| start + 1)
                .unwrap_or(1);

            let line = color.format_search_line(
                &path_str,
                line_match.line_number,
                col,
                &line_match.content,
                &line_match.ranges,
            );

            if writer.write_line(&line).is_err() {
                break;
            }
        }
    }
    let _ = writer.finish();

    Ok(ExitCode::Success)
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexrs_core::SegmentManager;
    use indexrs_core::segment::InputFile;
    use std::path::Path;

    fn build_test_index(dir: &Path) -> SegmentManager {
        let indexrs_dir = dir.join(".indexrs");
        std::fs::create_dir_all(indexrs_dir.join("segments")).unwrap();
        let manager = SegmentManager::new(&indexrs_dir).unwrap();
        manager
            .index_files(vec![
                InputFile {
                    path: "src/main.rs".to_string(),
                    content: b"fn main() {\n    println!(\"hello world\");\n}\n".to_vec(),
                    mtime: 100,
                },
                InputFile {
                    path: "src/lib.rs".to_string(),
                    content: b"pub fn greeting() -> &'static str {\n    \"hello\"\n}\n".to_vec(),
                    mtime: 200,
                },
            ])
            .unwrap();
        manager
    }

    #[test]
    fn test_resolve_match_pattern_literal() {
        let pattern = resolve_match_pattern("hello", false, false, true, false);
        assert!(matches!(pattern, MatchPattern::LiteralCaseInsensitive(_)));
    }

    #[test]
    fn test_resolve_match_pattern_case_sensitive() {
        let pattern = resolve_match_pattern("hello", false, true, false, false);
        assert!(matches!(pattern, MatchPattern::Literal(_)));
    }

    #[test]
    fn test_resolve_match_pattern_regex() {
        let pattern = resolve_match_pattern("fn\\s+", true, false, false, false);
        assert!(matches!(pattern, MatchPattern::Regex(_)));
    }

    #[test]
    fn test_resolve_match_pattern_smart_case_lower() {
        let pattern = resolve_match_pattern("hello", false, false, false, true);
        assert!(matches!(pattern, MatchPattern::LiteralCaseInsensitive(_)));
    }

    #[test]
    fn test_resolve_match_pattern_smart_case_upper() {
        let pattern = resolve_match_pattern("Hello", false, false, false, true);
        assert!(matches!(pattern, MatchPattern::Literal(_)));
    }

    #[test]
    fn test_search_vimgrep_format() {
        let dir = tempfile::tempdir().unwrap();
        let manager = build_test_index(dir.path());
        let snapshot = manager.snapshot();

        let mut buf = Vec::new();
        let color = ColorConfig::new(false);

        let opts = SearchCmdOptions {
            pattern: MatchPattern::LiteralCaseInsensitive("println".to_string()),
            context_lines: 0,
            limit: 1000,
            language: None,
            path_glob: None,
            stats: false,
        };

        let exit = {
            let mut writer = StreamingWriter::new(&mut buf);
            run_search(
                &snapshot,
                &opts,
                &color,
                &PathRewriter::identity(),
                &mut writer,
            )
            .unwrap()
        };
        let output = String::from_utf8(buf).unwrap();

        assert!(output.contains("src/main.rs:2:"));
        assert!(output.contains("println"));
        assert!(matches!(exit, ExitCode::Success));
    }

    #[test]
    fn test_search_no_results() {
        let dir = tempfile::tempdir().unwrap();
        let manager = build_test_index(dir.path());
        let snapshot = manager.snapshot();

        let mut buf = Vec::new();
        let color = ColorConfig::new(false);

        let opts = SearchCmdOptions {
            pattern: MatchPattern::LiteralCaseInsensitive("nonexistent_string_xyz".to_string()),
            context_lines: 0,
            limit: 1000,
            language: None,
            path_glob: None,
            stats: false,
        };

        let exit = {
            let mut writer = StreamingWriter::new(&mut buf);
            run_search(
                &snapshot,
                &opts,
                &color,
                &PathRewriter::identity(),
                &mut writer,
            )
            .unwrap()
        };
        assert!(matches!(exit, ExitCode::NoResults));
    }

    #[test]
    fn test_search_streaming_vimgrep_format() {
        let dir = tempfile::tempdir().unwrap();
        let manager = build_test_index(dir.path());
        let snapshot = manager.snapshot();

        let mut buf = Vec::new();
        let color = ColorConfig::new(false);

        let opts = SearchCmdOptions {
            pattern: MatchPattern::LiteralCaseInsensitive("println".to_string()),
            context_lines: 0,
            limit: 1000,
            language: None,
            path_glob: None,
            stats: false,
        };

        let exit = {
            let mut writer = StreamingWriter::new(&mut buf);
            run_search_streaming(
                &snapshot,
                &opts,
                &color,
                &PathRewriter::identity(),
                &mut writer,
            )
            .unwrap()
        };
        let output = String::from_utf8(buf).unwrap();

        assert!(output.contains("src/main.rs:2:"));
        assert!(output.contains("println"));
        assert!(matches!(exit, ExitCode::Success));
    }

    #[test]
    fn test_search_streaming_no_results() {
        let dir = tempfile::tempdir().unwrap();
        let manager = build_test_index(dir.path());
        let snapshot = manager.snapshot();

        let mut buf = Vec::new();
        let color = ColorConfig::new(false);

        let opts = SearchCmdOptions {
            pattern: MatchPattern::LiteralCaseInsensitive("nonexistent_string_xyz".to_string()),
            context_lines: 0,
            limit: 1000,
            language: None,
            path_glob: None,
            stats: false,
        };

        let exit = {
            let mut writer = StreamingWriter::new(&mut buf);
            run_search_streaming(
                &snapshot,
                &opts,
                &color,
                &PathRewriter::identity(),
                &mut writer,
            )
            .unwrap()
        };
        assert!(matches!(exit, ExitCode::NoResults));
    }

    #[test]
    fn test_search_rewrites_paths_to_cwd_relative() {
        let dir = tempfile::tempdir().unwrap();
        let manager = build_test_index(dir.path());
        let snapshot = manager.snapshot();

        let mut buf = Vec::new();
        let color = ColorConfig::new(false);
        // Simulate CWD = repo/src (inside repo)
        let rewriter = PathRewriter::new(Path::new("/repo"), Path::new("/repo/src"));

        let opts = SearchCmdOptions {
            pattern: MatchPattern::LiteralCaseInsensitive("println".to_string()),
            context_lines: 0,
            limit: 1000,
            language: None,
            path_glob: None,
            stats: false,
        };

        let exit = {
            let mut writer = StreamingWriter::new(&mut buf);
            run_search(&snapshot, &opts, &color, &rewriter, &mut writer).unwrap()
        };
        let output = String::from_utf8(buf).unwrap();

        // "src/main.rs" should become "main.rs" with CWD = /repo/src
        assert!(
            output.contains("main.rs:2:"),
            "expected rewritten path, got: {output}"
        );
        assert!(
            !output.contains("src/main.rs:"),
            "path should not have src/ prefix"
        );
        assert!(matches!(exit, ExitCode::Success));
    }

    #[test]
    fn test_flags_to_query_case_insensitive() {
        let pattern = MatchPattern::LiteralCaseInsensitive("hello".to_string());
        let query = flags_to_query(&pattern, None).unwrap();
        assert_eq!(
            query,
            Query::Literal(LiteralQuery {
                text: "hello".to_string(),
                case_sensitive: false,
            })
        );
    }

    #[test]
    fn test_flags_to_query_case_sensitive() {
        let pattern = MatchPattern::Literal("Hello".to_string());
        let query = flags_to_query(&pattern, None).unwrap();
        assert_eq!(
            query,
            Query::Literal(LiteralQuery {
                text: "Hello".to_string(),
                case_sensitive: true,
            })
        );
    }

    #[test]
    fn test_flags_to_query_regex() {
        let pattern = MatchPattern::Regex("fn\\s+".to_string());
        let query = flags_to_query(&pattern, None).unwrap();
        assert_eq!(
            query,
            Query::Regex(RegexQuery {
                pattern: "fn\\s+".to_string(),
                case_sensitive: true,
            })
        );
    }

    #[test]
    fn test_flags_to_query_with_language() {
        let pattern = MatchPattern::LiteralCaseInsensitive("println".to_string());
        let query = flags_to_query(&pattern, Some("rust")).unwrap();
        if let Query::And(children) = &query {
            assert_eq!(children.len(), 2);
            assert!(matches!(children[0], Query::LanguageFilter(_)));
        } else {
            panic!("expected And, got {query:?}");
        }
    }

    #[test]
    fn test_flags_to_query_unknown_language() {
        let pattern = MatchPattern::LiteralCaseInsensitive("hello".to_string());
        assert!(flags_to_query(&pattern, Some("brainfuck")).is_err());
    }
}
