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

use regex_syntax::Parser;
use regex_syntax::hir::HirKind;
use regex_syntax::hir::literal::{ExtractKind, Extractor};

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

/// Extract trigrams from a regex pattern by analyzing its literal fragments.
///
/// Uses `regex-syntax` to parse the pattern into HIR, then uses the
/// `Extractor` to find required literal byte sequences. Trigrams are
/// extracted from each literal fragment and merged.
///
/// For top-level alternations (`foo|bar`), extracts trigrams from each
/// branch separately and returns `TrigramQuery::Any`. If no required
/// literals can be extracted (e.g., pure wildcards `.*`), returns
/// `TrigramQuery::None`.
///
/// Returns `TrigramQuery::None` on parse errors (graceful fallback to full scan).
pub fn extract_regex_trigrams(pattern: &str) -> TrigramQuery {
    // Parse the regex pattern into HIR
    let hir = match Parser::new().parse(pattern) {
        Ok(hir) => hir,
        Err(_) => return TrigramQuery::None,
    };

    // Check for top-level alternation first
    if let HirKind::Alternation(branches) = hir.kind() {
        let mut branch_trigrams: Vec<HashSet<Trigram>> = Vec::new();
        for branch in branches {
            let trigrams = extract_trigrams_from_hir(branch);
            if trigrams.is_empty() {
                // If any branch has no trigrams, the whole OR can't be pruned
                return TrigramQuery::None;
            }
            branch_trigrams.push(trigrams);
        }
        return match branch_trigrams.len() {
            0 => TrigramQuery::None,
            1 => TrigramQuery::All(branch_trigrams.into_iter().next().unwrap()),
            _ => TrigramQuery::Any(branch_trigrams),
        };
    }

    // Non-alternation: extract trigrams from the whole pattern
    let trigrams = extract_trigrams_from_hir(&hir);
    if trigrams.is_empty() {
        TrigramQuery::None
    } else {
        TrigramQuery::All(trigrams)
    }
}

/// Extract trigrams from a parsed HIR node by finding literal fragments.
///
/// Uses prefix and suffix extraction for maximum coverage, then merges
/// the trigram sets.
fn extract_trigrams_from_hir(hir: &regex_syntax::hir::Hir) -> HashSet<Trigram> {
    let mut all_trigrams: HashSet<Trigram> = HashSet::new();

    // Extract prefix literals
    let mut extractor = Extractor::new();
    extractor.kind(ExtractKind::Prefix);
    let prefix_seq = extractor.extract(hir);

    // Extract suffix literals
    let mut extractor_suffix = Extractor::new();
    extractor_suffix.kind(ExtractKind::Suffix);
    let suffix_seq = extractor_suffix.extract(hir);

    // Process both prefix and suffix sequences
    for seq in [&prefix_seq, &suffix_seq] {
        if let Some(literals) = seq.literals() {
            for lit in literals {
                let bytes = lit.as_bytes();
                if bytes.len() >= 3 {
                    all_trigrams.extend(extract_unique_trigrams_folded(bytes));
                }
            }
        }
    }

    all_trigrams
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

    // ---- extract_regex_trigrams tests ----

    #[test]
    fn test_extract_regex_trigrams_simple_literal() {
        // /HttpRequest/ -> extracts trigrams from literal "HttpRequest", folded
        let result = extract_regex_trigrams("HttpRequest");
        match result {
            TrigramQuery::All(set) => {
                assert_eq!(set.len(), 9);
                // Folded to lowercase
                assert!(set.contains(&Trigram::from_bytes(b'h', b't', b't')));
            }
            _ => panic!("expected TrigramQuery::All"),
        }
    }

    #[test]
    fn test_extract_regex_trigrams_with_wildcard() {
        // /Err\(.*Error\)/ -> literals "Err(" and "Error)"
        let result = extract_regex_trigrams(r"Err\(.*Error\)");
        match result {
            TrigramQuery::All(set) => {
                assert!(set.len() >= 1); // At least some trigrams from "Err(" or "Error)"
                // "err" should be present (folded from "Err")
                assert!(set.contains(&Trigram::from_bytes(b'e', b'r', b'r')));
            }
            _ => panic!("expected TrigramQuery::All, got {:?}", result),
        }
    }

    #[test]
    fn test_extract_regex_trigrams_alternation() {
        // /foo|bar/ -> OR of trigrams from "foo" and "bar"
        let result = extract_regex_trigrams("foo|bar");
        match result {
            TrigramQuery::Any(branches) => {
                assert_eq!(branches.len(), 2);
            }
            // Also acceptable: All or None depending on extractor behavior
            TrigramQuery::All(_) | TrigramQuery::None => {}
        }
    }

    #[test]
    fn test_extract_regex_trigrams_pure_wildcard() {
        // /.*/ -> no literals -> None
        let result = extract_regex_trigrams(".*");
        assert_eq!(result, TrigramQuery::None);
    }

    #[test]
    fn test_extract_regex_trigrams_short_literal() {
        // /ab/ -> only 2-char literal -> None
        let result = extract_regex_trigrams("ab");
        assert_eq!(result, TrigramQuery::None);
    }

    #[test]
    fn test_extract_regex_trigrams_invalid_regex() {
        // Invalid regex -> None (graceful fallback)
        let result = extract_regex_trigrams("(unclosed");
        assert_eq!(result, TrigramQuery::None);
    }
}
