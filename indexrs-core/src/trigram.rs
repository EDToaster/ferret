//! Trigram extraction from byte content.
//!
//! This module provides functions to extract trigrams (3-byte sequences) from
//! file content. Trigrams are the fundamental indexing unit: every 3-byte window
//! position in a file produces a [`Trigram`] that is recorded in the posting lists.
//!
//! Extraction is byte-level, not character-level. UTF-8 multi-byte sequences that
//! span trigram boundaries are handled naturally since we operate on raw bytes.

use std::collections::HashSet;

use crate::types::Trigram;

/// Extract all trigrams from the given byte content by sliding a 3-byte window.
///
/// Returns an iterator yielding one [`Trigram`] for each window position.
/// Content shorter than 3 bytes produces no trigrams.
///
/// # Examples
///
/// ```
/// use indexrs_core::trigram::extract_trigrams;
/// use indexrs_core::Trigram;
///
/// let content = b"abc";
/// let trigrams: Vec<Trigram> = extract_trigrams(content).collect();
/// assert_eq!(trigrams, vec![Trigram::from_bytes(b'a', b'b', b'c')]);
/// ```
pub fn extract_trigrams(content: &[u8]) -> impl Iterator<Item = Trigram> + '_ {
    content
        .windows(3)
        .map(|w| Trigram::from_bytes(w[0], w[1], w[2]))
}

/// Extract the unique set of trigrams from the given byte content.
///
/// This is equivalent to collecting [`extract_trigrams`] into a [`HashSet`],
/// removing duplicate trigrams that appear at multiple positions.
///
/// # Examples
///
/// ```
/// use indexrs_core::trigram::extract_unique_trigrams;
/// use indexrs_core::Trigram;
///
/// let content = b"abab";
/// let unique = extract_unique_trigrams(content);
/// assert_eq!(unique.len(), 2); // "aba" and "bab"
/// ```
pub fn extract_unique_trigrams(content: &[u8]) -> HashSet<Trigram> {
    extract_trigrams(content).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_trigrams_empty() {
        let trigrams: Vec<Trigram> = extract_trigrams(b"").collect();
        assert!(trigrams.is_empty());
    }

    #[test]
    fn test_extract_trigrams_one_byte() {
        let trigrams: Vec<Trigram> = extract_trigrams(b"a").collect();
        assert!(trigrams.is_empty());
    }

    #[test]
    fn test_extract_trigrams_two_bytes() {
        let trigrams: Vec<Trigram> = extract_trigrams(b"ab").collect();
        assert!(trigrams.is_empty());
    }

    #[test]
    fn test_extract_trigrams_three_bytes() {
        let trigrams: Vec<Trigram> = extract_trigrams(b"abc").collect();
        assert_eq!(trigrams.len(), 1);
        assert_eq!(trigrams[0], Trigram::from_bytes(b'a', b'b', b'c'));
    }

    #[test]
    fn test_extract_trigrams_fn_main() {
        // From Appendix A: File 0 is "fn main() {}"
        // Trigrams at each offset:
        //   0: "fn "
        //   1: "n m"
        //   2: " ma"
        //   3: "mai"
        //   4: "ain"
        //   5: "in("
        //   6: "n()"
        //   7: "() "
        //   8: ") {"
        //   9: " {}"
        let content = b"fn main() {}";
        let trigrams: Vec<Trigram> = extract_trigrams(content).collect();

        assert_eq!(trigrams.len(), 10);

        let expected = vec![
            Trigram::from_bytes(b'f', b'n', b' '), // offset 0: "fn "
            Trigram::from_bytes(b'n', b' ', b'm'), // offset 1: "n m"
            Trigram::from_bytes(b' ', b'm', b'a'), // offset 2: " ma"
            Trigram::from_bytes(b'm', b'a', b'i'), // offset 3: "mai"
            Trigram::from_bytes(b'a', b'i', b'n'), // offset 4: "ain"
            Trigram::from_bytes(b'i', b'n', b'('), // offset 5: "in("
            Trigram::from_bytes(b'n', b'(', b')'), // offset 6: "n()"
            Trigram::from_bytes(b'(', b')', b' '), // offset 7: "() "
            Trigram::from_bytes(b')', b' ', b'{'), // offset 8: ") {"
            Trigram::from_bytes(b' ', b'{', b'}'), // offset 9: " {}"
        ];

        assert_eq!(trigrams, expected);
    }

    #[test]
    fn test_extract_trigrams_count_formula() {
        // For content of length N, there are N-2 trigrams (when N >= 3)
        let content = b"abcdefgh"; // length 8 -> 6 trigrams
        let trigrams: Vec<Trigram> = extract_trigrams(content).collect();
        assert_eq!(trigrams.len(), 6);
    }

    #[test]
    fn test_extract_unique_trigrams_deduplicates() {
        // "aaaa" has trigrams: "aaa", "aaa" — only 1 unique
        let content = b"aaaa";
        let unique = extract_unique_trigrams(content);
        assert_eq!(unique.len(), 1);
        assert!(unique.contains(&Trigram::from_bytes(b'a', b'a', b'a')));
    }

    #[test]
    fn test_extract_unique_trigrams_fn_main() {
        // "fn main() {}" has 10 trigrams, all distinct (from Appendix A)
        let content = b"fn main() {}";
        let unique = extract_unique_trigrams(content);
        assert_eq!(unique.len(), 10);

        // Verify all expected trigrams are present
        assert!(unique.contains(&Trigram::from_bytes(b'f', b'n', b' ')));
        assert!(unique.contains(&Trigram::from_bytes(b'n', b' ', b'm')));
        assert!(unique.contains(&Trigram::from_bytes(b' ', b'm', b'a')));
        assert!(unique.contains(&Trigram::from_bytes(b'm', b'a', b'i')));
        assert!(unique.contains(&Trigram::from_bytes(b'a', b'i', b'n')));
        assert!(unique.contains(&Trigram::from_bytes(b'i', b'n', b'(')));
        assert!(unique.contains(&Trigram::from_bytes(b'n', b'(', b')')));
        assert!(unique.contains(&Trigram::from_bytes(b'(', b')', b' ')));
        assert!(unique.contains(&Trigram::from_bytes(b')', b' ', b'{')));
        assert!(unique.contains(&Trigram::from_bytes(b' ', b'{', b'}')));
    }

    #[test]
    fn test_extract_unique_trigrams_empty() {
        let unique = extract_unique_trigrams(b"");
        assert!(unique.is_empty());
    }

    #[test]
    fn test_extract_trigrams_non_ascii_bytes() {
        // Trigram extraction works on raw bytes, including non-ASCII
        let content: &[u8] = &[0xFF, 0x00, 0x80, 0x7F];
        let trigrams: Vec<Trigram> = extract_trigrams(content).collect();
        assert_eq!(trigrams.len(), 2);
        assert_eq!(trigrams[0], Trigram::from_bytes(0xFF, 0x00, 0x80));
        assert_eq!(trigrams[1], Trigram::from_bytes(0x00, 0x80, 0x7F));
    }
}
