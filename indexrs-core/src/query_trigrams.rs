//! Trigram extraction from parsed query ASTs.
//!
//! This module converts a [`Query`] AST into a [`TrigramQuery`] that describes
//! which trigrams to look up in the index. The query planner uses the extracted
//! trigrams to fetch posting lists and build an execution plan.
//!
//! # Extraction Strategy
//!
//! - **Literal/Phrase queries**: Extract all trigrams directly from the search string.
//!   All trigrams must match (AND semantics).
//! - **Regex queries**: Parse the regex with `regex-syntax`, extract required literal
//!   fragments from the HIR, then extract trigrams from those fragments.
//! - **AND queries**: Merge (union) trigram sets from all children -- a file must
//!   contain trigrams from every child.
//! - **OR queries**: Keep trigram sets separate -- a file matching any branch suffices.
//! - **NOT queries**: Cannot use trigrams for pruning (negation inverts the set).
//! - **Filter queries** (Path, Language): These don't produce content trigrams;
//!   they're handled by other index types.
//!
//! When no trigrams can be extracted (short queries, wildcard-only regex, NOT-only
//! queries), the result is [`TrigramQuery::None`], signaling the planner to fall
//! back to a full file scan.
//!
//! # Case Folding
//!
//! All trigram extraction produces **lowercase (ASCII-folded) trigrams** to match
//! the case-folded index. The `case_sensitive` flag on query types does NOT affect
//! trigram extraction -- it only affects verification (HHC-49).

use std::collections::HashSet;

use crate::trigram::extract_unique_trigrams_folded;
use crate::types::Trigram;

/// Describes the trigram lookup strategy for a parsed query.
///
/// The query planner uses this to decide how to query the trigram index:
/// - `All`: intersect posting lists for all trigrams (AND semantics)
/// - `Any`: union the results of multiple `All` sets (OR semantics)
/// - `None`: no trigrams available, must scan all files
#[derive(Debug, Clone, PartialEq)]
pub enum TrigramQuery {
    /// All trigrams must be present in a file (AND intersection).
    /// Used for literal, phrase, and AND-combined queries.
    All(HashSet<Trigram>),

    /// At least one branch's trigram set must match (OR union).
    /// Each inner `HashSet<Trigram>` is an AND-set; the outer Vec is OR'd.
    /// A file is a candidate if it matches ALL trigrams in ANY one branch.
    Any(Vec<HashSet<Trigram>>),

    /// No trigrams could be extracted. The planner must fall back to
    /// scanning all files. This occurs for:
    /// - Queries shorter than 3 characters
    /// - Regex patterns with no required literal substrings (e.g., `.*`)
    /// - NOT-only queries
    /// - Filter-only queries (PathFilter, LanguageFilter)
    None,
}

impl TrigramQuery {
    /// Returns `true` if this is `TrigramQuery::None`.
    pub fn is_none(&self) -> bool {
        matches!(self, TrigramQuery::None)
    }

    /// Returns the total number of unique trigrams across all branches.
    /// Useful for cost estimation in the query planner.
    pub fn trigram_count(&self) -> usize {
        match self {
            TrigramQuery::All(set) => set.len(),
            TrigramQuery::Any(branches) => branches.iter().map(|s| s.len()).sum(),
            TrigramQuery::None => 0,
        }
    }
}

/// Extract trigrams from a literal search string.
///
/// Always produces lowercase (ASCII-folded) trigrams to match the case-folded
/// index. The `case_sensitive` flag on query types affects verification only,
/// not trigram extraction.
///
/// Returns `TrigramQuery::All` with the unique trigram set if the string
/// is at least 3 bytes long, or `TrigramQuery::None` if too short.
pub fn extract_literal_trigrams(text: &str) -> TrigramQuery {
    let bytes = text.as_bytes();
    if bytes.len() < 3 {
        return TrigramQuery::None;
    }
    let trigrams = extract_unique_trigrams_folded(bytes);
    if trigrams.is_empty() {
        TrigramQuery::None
    } else {
        TrigramQuery::All(trigrams)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Trigram;

    #[test]
    fn test_trigram_query_none_is_none() {
        assert!(TrigramQuery::None.is_none());
    }

    #[test]
    fn test_trigram_query_all_is_not_none() {
        let mut set = HashSet::new();
        set.insert(Trigram::from_bytes(b'a', b'b', b'c'));
        assert!(!TrigramQuery::All(set).is_none());
    }

    #[test]
    fn test_trigram_query_count_all() {
        let mut set = HashSet::new();
        set.insert(Trigram::from_bytes(b'a', b'b', b'c'));
        set.insert(Trigram::from_bytes(b'b', b'c', b'd'));
        assert_eq!(TrigramQuery::All(set).trigram_count(), 2);
    }

    #[test]
    fn test_trigram_query_count_any() {
        let mut s1 = HashSet::new();
        s1.insert(Trigram::from_bytes(b'a', b'b', b'c'));
        let mut s2 = HashSet::new();
        s2.insert(Trigram::from_bytes(b'x', b'y', b'z'));
        s2.insert(Trigram::from_bytes(b'y', b'z', b'w'));
        assert_eq!(TrigramQuery::Any(vec![s1, s2]).trigram_count(), 3);
    }

    #[test]
    fn test_trigram_query_count_none() {
        assert_eq!(TrigramQuery::None.trigram_count(), 0);
    }

    // ---- extract_literal_trigrams tests ----

    #[test]
    fn test_extract_literal_trigrams_lowercase() {
        // "httprequest" -> 9 trigrams, all lowercase
        let result = extract_literal_trigrams("httprequest");
        match result {
            TrigramQuery::All(set) => {
                assert_eq!(set.len(), 9);
                assert!(set.contains(&Trigram::from_bytes(b'h', b't', b't')));
                assert!(set.contains(&Trigram::from_bytes(b'e', b's', b't')));
            }
            _ => panic!("expected TrigramQuery::All"),
        }
    }

    #[test]
    fn test_extract_literal_trigrams_mixed_case_folded() {
        // "HttpRequest" -> folded to lowercase trigrams
        // Same trigrams as "httprequest" since index is case-folded
        let result = extract_literal_trigrams("HttpRequest");
        match result {
            TrigramQuery::All(set) => {
                assert_eq!(set.len(), 9);
                // All trigrams are lowercase (folded)
                assert!(set.contains(&Trigram::from_bytes(b'h', b't', b't')));
                assert!(set.contains(&Trigram::from_bytes(b'e', b's', b't')));
                // No uppercase trigrams
                assert!(!set.contains(&Trigram::from_bytes(b'H', b't', b't')));
            }
            _ => panic!("expected TrigramQuery::All"),
        }
    }

    #[test]
    fn test_extract_literal_trigrams_short() {
        // "fn" is only 2 chars -> no trigrams -> None
        assert_eq!(extract_literal_trigrams("fn"), TrigramQuery::None);
    }

    #[test]
    fn test_extract_literal_trigrams_exact_three() {
        // "abc" -> exactly 1 trigram
        let result = extract_literal_trigrams("abc");
        match result {
            TrigramQuery::All(set) => {
                assert_eq!(set.len(), 1);
                assert!(set.contains(&Trigram::from_bytes(b'a', b'b', b'c')));
            }
            _ => panic!("expected TrigramQuery::All"),
        }
    }

    #[test]
    fn test_extract_literal_trigrams_empty() {
        assert_eq!(extract_literal_trigrams(""), TrigramQuery::None);
    }

    #[test]
    fn test_extract_literal_trigrams_deduplicates() {
        // "aaaa" has trigrams: "aaa", "aaa" -> deduplicated to 1
        let result = extract_literal_trigrams("aaaa");
        match result {
            TrigramQuery::All(set) => assert_eq!(set.len(), 1),
            _ => panic!("expected TrigramQuery::All"),
        }
    }
}
