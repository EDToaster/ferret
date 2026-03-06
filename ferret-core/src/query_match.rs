//! Recursive query AST verifier for content matching.
//!
//! [`QueryMatcher`] evaluates a [`Query`] AST against raw file content by
//! recursively walking the AST. Leaf nodes (literals, phrases, regexes) are
//! verified using [`ContentVerifier`], while boolean nodes (AND, OR, NOT) apply
//! the appropriate set logic. Metadata-level filters (path, language) pass
//! through as "no content constraint."

use crate::query::Query;
use crate::search::{ContextLine, LineMatch, MatchPattern};
use crate::verify::ContentVerifier;

/// Evaluates a [`Query`] AST against raw file content.
///
/// Reuses [`ContentVerifier`] for leaf-node matching and adds recursive boolean
/// logic for AND, OR, and NOT nodes.
pub struct QueryMatcher<'a> {
    query: &'a Query,
    context_lines: u32,
}

impl<'a> QueryMatcher<'a> {
    /// Create a new `QueryMatcher` for the given query and context line count.
    pub fn new(query: &'a Query, context_lines: u32) -> Self {
        Self {
            query,
            context_lines,
        }
    }

    /// Match the query against content. Returns `Some(lines)` if the file
    /// matches, or `None` if no match.
    ///
    /// When `context_lines > 0`, populates `context_before` / `context_after`
    /// on each `LineMatch` with surrounding source lines.
    pub fn matches(&self, content: &[u8]) -> Option<Vec<LineMatch>> {
        let lines = self.eval(self.query, content)?;
        if self.context_lines == 0 || lines.is_empty() {
            return Some(lines);
        }
        Some(attach_context(content, lines, self.context_lines))
    }

    /// Recursively evaluate a query node against content.
    ///
    /// Context lines are NOT computed here — `matches()` attaches them once
    /// after the full AST has been evaluated so we avoid redundant line-index
    /// scans in every leaf node.
    fn eval(&self, query: &Query, content: &[u8]) -> Option<Vec<LineMatch>> {
        match query {
            Query::Literal(lit) => {
                let pattern = if lit.case_sensitive {
                    MatchPattern::Literal(lit.text.clone())
                } else {
                    MatchPattern::LiteralCaseInsensitive(lit.text.clone())
                };
                let verifier = ContentVerifier::new(pattern, 0);
                let lines = verifier.verify(content);
                if lines.is_empty() { None } else { Some(lines) }
            }
            Query::Phrase(ph) => {
                let pattern = if ph.case_sensitive {
                    MatchPattern::Literal(ph.text.clone())
                } else {
                    MatchPattern::LiteralCaseInsensitive(ph.text.clone())
                };
                let verifier = ContentVerifier::new(pattern, 0);
                let lines = verifier.verify(content);
                if lines.is_empty() { None } else { Some(lines) }
            }
            Query::Regex(re) => {
                let effective_pattern = if re.case_sensitive {
                    re.pattern.clone()
                } else {
                    format!("(?i){}", re.pattern)
                };
                let pattern = MatchPattern::Regex(effective_pattern);
                let verifier = ContentVerifier::new(pattern, 0);
                let lines = verifier.verify(content);
                if lines.is_empty() { None } else { Some(lines) }
            }
            Query::PathFilter(_) | Query::LanguageFilter(_) => {
                // Metadata-level filters have no content constraint.
                Some(vec![])
            }
            Query::Not(inner) => {
                let inner_result = self.eval(inner, content);
                match inner_result {
                    Some(_) => None,
                    None => Some(vec![]),
                }
            }
            Query::Or(left, right) => {
                let left_result = self.eval(left, content);
                let right_result = self.eval(right, content);
                match (left_result, right_result) {
                    (None, None) => None,
                    (Some(lines), None) | (None, Some(lines)) => Some(lines),
                    (Some(left_lines), Some(right_lines)) => {
                        Some(merge_line_matches(left_lines, right_lines))
                    }
                }
            }
            Query::And(children) => {
                let mut merged: Vec<LineMatch> = Vec::new();
                for child in children {
                    match self.eval(child, content) {
                        None => return None,
                        Some(lines) => {
                            merged.extend(lines);
                        }
                    }
                }
                // Sort and dedup
                merged.sort_by_key(|m| m.line_number);
                merged.dedup_by_key(|m| m.line_number);
                Some(merged)
            }
        }
    }
}

/// Merge two sets of line matches, sort by line number, and deduplicate.
fn merge_line_matches(mut left: Vec<LineMatch>, right: Vec<LineMatch>) -> Vec<LineMatch> {
    left.extend(right);
    left.sort_by_key(|m| m.line_number);
    left.dedup_by_key(|m| m.line_number);
    left
}

/// Post-process matched lines to attach context (before/after) lines.
///
/// Groups nearby matches whose context windows overlap into blocks,
/// then attaches `context_before` to the first match in each block
/// and `context_after` to the last.
fn attach_context(content: &[u8], mut lines: Vec<LineMatch>, context_lines: u32) -> Vec<LineMatch> {
    if content.is_empty() || lines.is_empty() {
        return lines;
    }

    // Build a simple line table for content lookup.
    let newline_offsets: Vec<usize> = memchr::memchr_iter(b'\n', content).collect();
    let total_lines = if newline_offsets.last() == Some(&(content.len() - 1)) {
        newline_offsets.len() as u32
    } else {
        newline_offsets.len() as u32 + 1
    };

    let line_content_at = |line_num: u32| -> String {
        let line_0 = (line_num - 1) as usize;
        let start = if line_0 == 0 {
            0
        } else {
            newline_offsets[line_0 - 1] + 1
        };
        let end = if line_0 < newline_offsets.len() {
            newline_offsets[line_0]
        } else {
            content.len()
        };
        let s = std::str::from_utf8(&content[start..end]).unwrap_or("");
        s.strip_suffix('\r').unwrap_or(s).to_string()
    };

    lines.sort_by_key(|m| m.line_number);

    // Group matches into blocks where context windows overlap.
    let mut groups: Vec<Vec<usize>> = Vec::new(); // indices into `lines`
    for i in 0..lines.len() {
        let should_merge = groups.last().is_some_and(|group| {
            let last_line = lines[*group.last().unwrap()].line_number;
            (lines[i].line_number as u64) <= (last_line as u64) + 2 * (context_lines as u64) + 1
        });
        if should_merge {
            groups.last_mut().unwrap().push(i);
        } else {
            groups.push(vec![i]);
        }
    }

    // Attach context to the first and last match in each group.
    for group in &groups {
        let first_idx = group[0];
        let last_idx = *group.last().unwrap();

        let first_line = lines[first_idx].line_number;
        let last_line = lines[last_idx].line_number;

        let before_start = first_line.saturating_sub(context_lines).max(1);
        lines[first_idx].context_before = (before_start..first_line)
            .map(|ln| ContextLine {
                line_number: ln,
                content: line_content_at(ln),
                highlight_tokens: vec![],
            })
            .collect();

        let after_end = (last_line + context_lines).min(total_lines);
        lines[last_idx].context_after = ((last_line + 1)..=after_end)
            .map(|ln| ContextLine {
                line_number: ln,
                content: line_content_at(ln),
                highlight_tokens: vec![],
            })
            .collect();
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{LiteralQuery, PhraseQuery, RegexQuery};
    use crate::types::Language;

    #[test]
    fn test_literal_match() {
        let query = Query::Literal(LiteralQuery {
            text: "hello".to_string(),
            case_sensitive: false,
        });
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"say hello world\n";
        let result = matcher.matches(content);
        assert!(result.is_some());
        let lines = result.unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_number, 1);
        assert!(lines[0].content.contains("hello"));
    }

    #[test]
    fn test_literal_no_match() {
        let query = Query::Literal(LiteralQuery {
            text: "foobar".to_string(),
            case_sensitive: false,
        });
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"nothing relevant here\n";
        let result = matcher.matches(content);
        assert!(result.is_none());
    }

    #[test]
    fn test_and_both_match() {
        let query = Query::And(vec![
            Query::Literal(LiteralQuery {
                text: "hello".to_string(),
                case_sensitive: false,
            }),
            Query::Literal(LiteralQuery {
                text: "world".to_string(),
                case_sensitive: false,
            }),
        ]);
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"hello there\nworld here\n";
        let result = matcher.matches(content);
        assert!(result.is_some());
        let lines = result.unwrap();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_and_one_missing() {
        let query = Query::And(vec![
            Query::Literal(LiteralQuery {
                text: "hello".to_string(),
                case_sensitive: false,
            }),
            Query::Literal(LiteralQuery {
                text: "nonexistent".to_string(),
                case_sensitive: false,
            }),
        ]);
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"hello there\n";
        let result = matcher.matches(content);
        assert!(result.is_none());
    }

    #[test]
    fn test_or_one_matches() {
        let query = Query::Or(
            Box::new(Query::Literal(LiteralQuery {
                text: "hello".to_string(),
                case_sensitive: false,
            })),
            Box::new(Query::Literal(LiteralQuery {
                text: "nonexistent".to_string(),
                case_sensitive: false,
            })),
        );
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"hello there\n";
        let result = matcher.matches(content);
        assert!(result.is_some());
        let lines = result.unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].content.contains("hello"));
    }

    #[test]
    fn test_or_neither_matches() {
        let query = Query::Or(
            Box::new(Query::Literal(LiteralQuery {
                text: "nonexistent".to_string(),
                case_sensitive: false,
            })),
            Box::new(Query::Literal(LiteralQuery {
                text: "alsonothere".to_string(),
                case_sensitive: false,
            })),
        );
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"hello world\n";
        let result = matcher.matches(content);
        assert!(result.is_none());
    }

    #[test]
    fn test_not_excludes() {
        let query = Query::Not(Box::new(Query::Literal(LiteralQuery {
            text: "hello".to_string(),
            case_sensitive: false,
        })));
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"hello world\n";
        let result = matcher.matches(content);
        assert!(result.is_none());
    }

    #[test]
    fn test_not_includes() {
        let query = Query::Not(Box::new(Query::Literal(LiteralQuery {
            text: "nonexistent".to_string(),
            case_sensitive: false,
        })));
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"hello world\n";
        let result = matcher.matches(content);
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_regex_match() {
        let query = Query::Regex(RegexQuery {
            pattern: r"fn\s+\w+".to_string(),
            case_sensitive: true,
        });
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"fn main() {}\nlet x = 1;\n";
        let result = matcher.matches(content);
        assert!(result.is_some());
        let lines = result.unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_number, 1);
    }

    #[test]
    fn test_phrase_match() {
        let query = Query::Phrase(PhraseQuery {
            text: "hello world".to_string(),
            case_sensitive: false,
        });
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"say Hello World to all\n";
        let result = matcher.matches(content);
        assert!(result.is_some());
        let lines = result.unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].content.contains("Hello World"));
    }

    #[test]
    fn test_language_filter_passes_through() {
        let query = Query::LanguageFilter(Language::Rust);
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"fn main() {}\n";
        let result = matcher.matches(content);
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_context_lines_literal() {
        let query = Query::Literal(LiteralQuery {
            text: "MATCH".to_string(),
            case_sensitive: false,
        });
        let matcher = QueryMatcher::new(&query, 2);
        let content = b"line 1\nline 2\nline 3\nMATCH here\nline 5\nline 6\nline 7\n";
        let result = matcher.matches(content);
        assert!(result.is_some());
        let lines = result.unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_number, 4);
        // 2 context lines before (lines 2, 3)
        assert_eq!(lines[0].context_before.len(), 2);
        assert_eq!(lines[0].context_before[0].line_number, 2);
        assert_eq!(lines[0].context_before[1].line_number, 3);
        // 2 context lines after (lines 5, 6)
        assert_eq!(lines[0].context_after.len(), 2);
        assert_eq!(lines[0].context_after[0].line_number, 5);
        assert_eq!(lines[0].context_after[1].line_number, 6);
    }

    #[test]
    fn test_context_lines_merges_nearby() {
        let query = Query::Literal(LiteralQuery {
            text: "MATCH".to_string(),
            case_sensitive: false,
        });
        let matcher = QueryMatcher::new(&query, 1);
        let content = b"line 1\nMATCH1\nline 3\nMATCH2\nline 5\n";
        let result = matcher.matches(content);
        assert!(result.is_some());
        let lines = result.unwrap();
        assert_eq!(lines.len(), 2);
        // First match gets context_before (line 1), no context_after (merged)
        assert_eq!(lines[0].context_before.len(), 1);
        assert_eq!(lines[0].context_before[0].line_number, 1);
        assert!(lines[0].context_after.is_empty());
        // Last match gets context_after (line 5), no context_before (merged)
        assert!(lines[1].context_before.is_empty());
        assert_eq!(lines[1].context_after.len(), 1);
        assert_eq!(lines[1].context_after[0].line_number, 5);
    }

    #[test]
    fn test_context_lines_zero() {
        let query = Query::Literal(LiteralQuery {
            text: "hello".to_string(),
            case_sensitive: false,
        });
        let matcher = QueryMatcher::new(&query, 0);
        let content = b"line 1\nhello world\nline 3\n";
        let result = matcher.matches(content);
        assert!(result.is_some());
        let lines = result.unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].context_before.is_empty());
        assert!(lines[0].context_after.is_empty());
    }

    #[test]
    fn test_complex_and_or_not() {
        // (println OR eprintln) AND NOT deprecated
        let query = Query::And(vec![
            Query::Or(
                Box::new(Query::Literal(LiteralQuery {
                    text: "println".to_string(),
                    case_sensitive: false,
                })),
                Box::new(Query::Literal(LiteralQuery {
                    text: "eprintln".to_string(),
                    case_sensitive: false,
                })),
            ),
            Query::Not(Box::new(Query::Literal(LiteralQuery {
                text: "deprecated".to_string(),
                case_sensitive: false,
            }))),
        ]);
        let matcher = QueryMatcher::new(&query, 0);

        // File with println but no "deprecated" -> matches
        let content1 = b"fn main() {\n    println!(\"hi\");\n}\n";
        let result1 = matcher.matches(content1);
        assert!(result1.is_some());

        // File with eprintln but also "deprecated" -> no match
        let content2 = b"// deprecated\neprintln!(\"error\");\n";
        let result2 = matcher.matches(content2);
        assert!(result2.is_none());

        // File with neither println nor eprintln -> no match
        let content3 = b"fn helper() {}\n";
        let result3 = matcher.matches(content3);
        assert!(result3.is_none());
    }
}
