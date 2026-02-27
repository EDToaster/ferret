//! Core identifier types and enums for indexrs.
//!
//! This module defines the fundamental types used throughout the indexing system:
//! file identifiers, trigrams for index lookups, segment identifiers, language
//! classification, and symbol kinds.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Unique identifier for an indexed file within the index.
///
/// File IDs are assigned sequentially during indexing and used as compact
/// references in posting lists and metadata tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FileId(pub u32);

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A 3-byte n-gram used for trigram index lookups.
///
/// Trigrams are the fundamental unit of the search index. Every 3-byte sequence
/// in indexed files is recorded as a trigram, enabling fast substring and regex
/// search via posting list intersection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Trigram(pub [u8; 3]);

impl Trigram {
    /// Construct a trigram from three individual bytes.
    pub fn from_bytes(a: u8, b: u8, c: u8) -> Self {
        Trigram([a, b, c])
    }

    /// Convert the trigram to a `u32` value for use as a hash key or array index.
    ///
    /// The encoding packs the three bytes into the lower 24 bits:
    /// `(byte0 << 16) | (byte1 << 8) | byte2`
    pub fn to_u32(self) -> u32 {
        (self.0[0] as u32) << 16 | (self.0[1] as u32) << 8 | self.0[2] as u32
    }
}

impl fmt::Display for Trigram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &b in &self.0 {
            if b.is_ascii_graphic() || b == b' ' {
                write!(f, "{}", b as char)?;
            } else {
                write!(f, "\\x{b:02x}")?;
            }
        }
        Ok(())
    }
}

/// Identifier for an index segment.
///
/// The index is composed of multiple immutable segments, each containing a
/// subset of indexed files. Segments are created on updates and periodically
/// compacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SegmentId(pub u32);

impl fmt::Display for SegmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Programming language classification for indexed files.
///
/// Language detection is used for `language:rust` style query filters and for
/// selecting the appropriate tree-sitter grammar for symbol extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
    C,
    Cpp,
    Java,
    Ruby,
    Shell,
    Markdown,
    Unknown,
}

impl Language {
    /// Detect language from a file extension string (without the leading dot).
    ///
    /// Returns `Language::Unknown` for unrecognized extensions.
    ///
    /// # Examples
    ///
    /// ```
    /// use indexrs_core::Language;
    ///
    /// assert_eq!(Language::from_extension("rs"), Language::Rust);
    /// assert_eq!(Language::from_extension("py"), Language::Python);
    /// assert_eq!(Language::from_extension("xyz"), Language::Unknown);
    /// ```
    pub fn from_extension(ext: &str) -> Language {
        match ext {
            "rs" => Language::Rust,
            "py" | "pyi" => Language::Python,
            "ts" | "tsx" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "go" => Language::Go,
            "c" | "h" => Language::C,
            "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" => Language::Cpp,
            "java" => Language::Java,
            "rb" => Language::Ruby,
            "sh" | "bash" | "zsh" | "fish" => Language::Shell,
            "md" | "markdown" => Language::Markdown,
            _ => Language::Unknown,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Language::Rust => write!(f, "Rust"),
            Language::Python => write!(f, "Python"),
            Language::TypeScript => write!(f, "TypeScript"),
            Language::JavaScript => write!(f, "JavaScript"),
            Language::Go => write!(f, "Go"),
            Language::C => write!(f, "C"),
            Language::Cpp => write!(f, "C++"),
            Language::Java => write!(f, "Java"),
            Language::Ruby => write!(f, "Ruby"),
            Language::Shell => write!(f, "Shell"),
            Language::Markdown => write!(f, "Markdown"),
            Language::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Kind of symbol extracted from source code via tree-sitter.
///
/// Used in the symbol index to classify symbol definitions, enabling
/// `symbol:parse_query` style searches filtered by kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Struct,
    Trait,
    Enum,
    Interface,
    Class,
    Method,
    Constant,
    Variable,
    Type,
    Module,
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SymbolKind::Function => write!(f, "Function"),
            SymbolKind::Struct => write!(f, "Struct"),
            SymbolKind::Trait => write!(f, "Trait"),
            SymbolKind::Enum => write!(f, "Enum"),
            SymbolKind::Interface => write!(f, "Interface"),
            SymbolKind::Class => write!(f, "Class"),
            SymbolKind::Method => write!(f, "Method"),
            SymbolKind::Constant => write!(f, "Constant"),
            SymbolKind::Variable => write!(f, "Variable"),
            SymbolKind::Type => write!(f, "Type"),
            SymbolKind::Module => write!(f, "Module"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_id_display() {
        assert_eq!(FileId(42).to_string(), "42");
        assert_eq!(FileId(0).to_string(), "0");
        assert_eq!(FileId(u32::MAX).to_string(), "4294967295");
    }

    #[test]
    fn test_file_id_equality_and_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(FileId(1));
        set.insert(FileId(2));
        set.insert(FileId(1)); // duplicate
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_trigram_from_bytes() {
        let t = Trigram::from_bytes(b'a', b'b', b'c');
        assert_eq!(t, Trigram([b'a', b'b', b'c']));
    }

    #[test]
    fn test_trigram_to_u32() {
        let t = Trigram::from_bytes(b'a', b'b', b'c');
        // 'a' = 0x61, 'b' = 0x62, 'c' = 0x63
        // (0x61 << 16) | (0x62 << 8) | 0x63 = 6382179
        let expected = (0x61u32 << 16) | (0x62u32 << 8) | 0x63u32;
        assert_eq!(t.to_u32(), expected);
        assert_eq!(t.to_u32(), 6_382_179);
    }

    #[test]
    fn test_trigram_to_u32_zero() {
        let t = Trigram::from_bytes(0, 0, 0);
        assert_eq!(t.to_u32(), 0);
    }

    #[test]
    fn test_trigram_to_u32_max() {
        let t = Trigram::from_bytes(0xFF, 0xFF, 0xFF);
        assert_eq!(t.to_u32(), 0x00FF_FFFF);
    }

    #[test]
    fn test_trigram_display_printable() {
        let t = Trigram::from_bytes(b'f', b'o', b'o');
        assert_eq!(t.to_string(), "foo");
    }

    #[test]
    fn test_trigram_display_non_printable() {
        let t = Trigram::from_bytes(0x00, b'a', 0xFF);
        assert_eq!(t.to_string(), "\\x00a\\xff");
    }

    #[test]
    fn test_segment_id_display() {
        assert_eq!(SegmentId(1).to_string(), "1");
        assert_eq!(SegmentId(9999).to_string(), "9999");
    }

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension("rs"), Language::Rust);
        assert_eq!(Language::from_extension("py"), Language::Python);
        assert_eq!(Language::from_extension("pyi"), Language::Python);
        assert_eq!(Language::from_extension("ts"), Language::TypeScript);
        assert_eq!(Language::from_extension("tsx"), Language::TypeScript);
        assert_eq!(Language::from_extension("js"), Language::JavaScript);
        assert_eq!(Language::from_extension("jsx"), Language::JavaScript);
        assert_eq!(Language::from_extension("mjs"), Language::JavaScript);
        assert_eq!(Language::from_extension("cjs"), Language::JavaScript);
        assert_eq!(Language::from_extension("go"), Language::Go);
        assert_eq!(Language::from_extension("c"), Language::C);
        assert_eq!(Language::from_extension("h"), Language::C);
        assert_eq!(Language::from_extension("cpp"), Language::Cpp);
        assert_eq!(Language::from_extension("cxx"), Language::Cpp);
        assert_eq!(Language::from_extension("cc"), Language::Cpp);
        assert_eq!(Language::from_extension("hpp"), Language::Cpp);
        assert_eq!(Language::from_extension("java"), Language::Java);
        assert_eq!(Language::from_extension("rb"), Language::Ruby);
        assert_eq!(Language::from_extension("sh"), Language::Shell);
        assert_eq!(Language::from_extension("bash"), Language::Shell);
        assert_eq!(Language::from_extension("zsh"), Language::Shell);
        assert_eq!(Language::from_extension("fish"), Language::Shell);
        assert_eq!(Language::from_extension("md"), Language::Markdown);
        assert_eq!(Language::from_extension("markdown"), Language::Markdown);
        assert_eq!(Language::from_extension("unknown"), Language::Unknown);
        assert_eq!(Language::from_extension(""), Language::Unknown);
        assert_eq!(Language::from_extension("xyz"), Language::Unknown);
    }

    #[test]
    fn test_language_display() {
        assert_eq!(Language::Rust.to_string(), "Rust");
        assert_eq!(Language::Python.to_string(), "Python");
        assert_eq!(Language::TypeScript.to_string(), "TypeScript");
        assert_eq!(Language::JavaScript.to_string(), "JavaScript");
        assert_eq!(Language::Go.to_string(), "Go");
        assert_eq!(Language::C.to_string(), "C");
        assert_eq!(Language::Cpp.to_string(), "C++");
        assert_eq!(Language::Java.to_string(), "Java");
        assert_eq!(Language::Ruby.to_string(), "Ruby");
        assert_eq!(Language::Shell.to_string(), "Shell");
        assert_eq!(Language::Markdown.to_string(), "Markdown");
        assert_eq!(Language::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn test_symbol_kind_display() {
        assert_eq!(SymbolKind::Function.to_string(), "Function");
        assert_eq!(SymbolKind::Struct.to_string(), "Struct");
        assert_eq!(SymbolKind::Trait.to_string(), "Trait");
        assert_eq!(SymbolKind::Enum.to_string(), "Enum");
        assert_eq!(SymbolKind::Interface.to_string(), "Interface");
        assert_eq!(SymbolKind::Class.to_string(), "Class");
        assert_eq!(SymbolKind::Method.to_string(), "Method");
        assert_eq!(SymbolKind::Constant.to_string(), "Constant");
        assert_eq!(SymbolKind::Variable.to_string(), "Variable");
        assert_eq!(SymbolKind::Type.to_string(), "Type");
        assert_eq!(SymbolKind::Module.to_string(), "Module");
    }
}
