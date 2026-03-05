# Syntax Highlight Indexing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Pre-compute syntax highlighting tokens during indexing and store them in a per-segment `highlights.zst` file, enabling zero-cost highlighted search results and file preview at serve time.

**Architecture:** Use syntect to tokenize each file during segment build (Phase 1, parallelized). Store a compact binary format — line-indexed RLE tokens (Format C from benchmarking) — in `highlights.zst` alongside `content.zst`. Each file gets an independently-compressed block of tokens with per-line offset index, enabling O(1) line lookup. The highlight store is optional — segments without it gracefully degrade to unhighlighted output.

**Tech Stack:** syntect 5 (Rust crate), zstd compression (existing), Format C binary encoding (line-indexed RLE with 16-category TokenKind enum)

---

### Task 1: Add `highlight.rs` module with TokenKind, encoding, and HighlightStoreWriter/Reader

**Files:**
- Create: `ferret-core/src/highlight.rs`
- Modify: `ferret-core/src/lib.rs` (add module + pub exports)
- Modify: `ferret-core/Cargo.toml` (add syntect dependency)
- Test: inline `#[cfg(test)] mod tests` in `highlight.rs`

**Step 1: Add syntect dependency**

In `ferret-core/Cargo.toml`, add to `[dependencies]`:
```toml
syntect = { version = "5", default-features = false, features = ["default-syntaxes", "default-themes", "parsing", "html"] }
```

Note: we need `default-syntaxes` for language grammars but can skip `regex-fancy` if `default-features = false` plus the above features covers it. Verify with a `cargo check`.

**Step 2: Write the failing test for TokenKind and RLE encoding**

Create `ferret-core/src/highlight.rs`:

```rust
/// Syntax highlighting token types and storage.
///
/// Tokens are classified into 16 categories (4 bits) and stored as
/// run-length encoded (len, kind) pairs with a per-line offset index.
/// This enables O(1) lookup of any line's highlighting tokens.

/// 16-category token classification. Fits in 4 bits (values 0–15).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Plain = 0,
    Keyword = 1,
    String = 2,
    Comment = 3,
    Number = 4,
    Function = 5,
    Type = 6,
    Variable = 7,
    Operator = 8,
    Punctuation = 9,
    Macro = 10,
    Attribute = 11,
    Constant = 12,
    Module = 13,
    Label = 14,
    Other = 15,
}

impl TokenKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Plain,
            1 => Self::Keyword,
            2 => Self::String,
            3 => Self::Comment,
            4 => Self::Number,
            5 => Self::Function,
            6 => Self::Type,
            7 => Self::Variable,
            8 => Self::Operator,
            9 => Self::Punctuation,
            10 => Self::Macro,
            11 => Self::Attribute,
            12 => Self::Constant,
            13 => Self::Module,
            14 => Self::Label,
            _ => Self::Other,
        }
    }
}

/// A single token: byte length + kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub len: usize,
    pub kind: TokenKind,
}

/// Per-file highlight data: line offsets into a flat RLE token buffer.
#[derive(Debug, Clone)]
pub struct FileHighlight {
    /// RLE-encoded tokens: (len: u16, kind: u8) triples, concatenated for all lines.
    pub token_data: Vec<u8>,
    /// Byte offset into `token_data` where each line's tokens start.
    /// Length = number of lines. The tokens for line `i` span from
    /// `line_offsets[i]` to `line_offsets[i+1]` (or end of `token_data`).
    pub line_offsets: Vec<u32>,
}

/// Encode tokens as RLE: adjacent same-kind tokens merged,
/// each run stored as (len: u16 LE, kind: u8) = 3 bytes.
pub fn encode_rle(tokens: &[Token]) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut iter = tokens.iter().peekable();
    while let Some(token) = iter.next() {
        let mut len = token.len;
        let kind = token.kind;
        // Merge adjacent same-kind
        while let Some(next) = iter.peek() {
            if next.kind == kind {
                len += next.len;
                iter.next();
            } else {
                break;
            }
        }
        // Split runs > u16::MAX
        while len > 0 {
            let chunk = len.min(u16::MAX as usize);
            buf.extend_from_slice(&(chunk as u16).to_le_bytes());
            buf.push(kind as u8);
            len -= chunk;
        }
    }
    buf
}

/// Decode RLE token data back into Token list.
pub fn decode_rle(data: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos + 2 < data.len() {
        let len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        let kind = TokenKind::from_u8(data[pos + 2]);
        tokens.push(Token { len, kind });
        pos += 3;
    }
    tokens
}

/// Build a `FileHighlight` from per-line token lists.
pub fn build_file_highlight(line_tokens: &[Vec<Token>]) -> FileHighlight {
    let mut token_data = Vec::new();
    let mut line_offsets = Vec::with_capacity(line_tokens.len());
    for tokens in line_tokens {
        line_offsets.push(token_data.len() as u32);
        let rle = encode_rle(tokens);
        token_data.extend_from_slice(&rle);
    }
    FileHighlight {
        token_data,
        line_offsets,
    }
}

impl FileHighlight {
    /// Get tokens for a specific line (0-indexed).
    pub fn tokens_for_line(&self, line: usize) -> Vec<Token> {
        if line >= self.line_offsets.len() {
            return Vec::new();
        }
        let start = self.line_offsets[line] as usize;
        let end = if line + 1 < self.line_offsets.len() {
            self.line_offsets[line + 1] as usize
        } else {
            self.token_data.len()
        };
        if start >= self.token_data.len() || start >= end {
            return Vec::new();
        }
        decode_rle(&self.token_data[start..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_kind_roundtrip() {
        for v in 0..=15u8 {
            let kind = TokenKind::from_u8(v);
            assert_eq!(kind as u8, v);
        }
        // Out of range maps to Other
        assert_eq!(TokenKind::from_u8(255), TokenKind::Other);
    }

    #[test]
    fn test_rle_encode_decode_roundtrip() {
        let tokens = vec![
            Token { len: 2, kind: TokenKind::Keyword },
            Token { len: 1, kind: TokenKind::Plain },
            Token { len: 4, kind: TokenKind::Function },
            Token { len: 2, kind: TokenKind::Punctuation },
        ];
        let encoded = encode_rle(&tokens);
        let decoded = decode_rle(&encoded);
        assert_eq!(tokens, decoded);
    }

    #[test]
    fn test_rle_merges_adjacent_same_kind() {
        let tokens = vec![
            Token { len: 3, kind: TokenKind::Plain },
            Token { len: 5, kind: TokenKind::Plain },
        ];
        let encoded = encode_rle(&tokens);
        let decoded = decode_rle(&encoded);
        // Should merge into a single (8, Plain)
        assert_eq!(decoded, vec![Token { len: 8, kind: TokenKind::Plain }]);
    }

    #[test]
    fn test_file_highlight_line_lookup() {
        let line0 = vec![
            Token { len: 2, kind: TokenKind::Keyword },
            Token { len: 5, kind: TokenKind::Function },
        ];
        let line1 = vec![
            Token { len: 10, kind: TokenKind::Comment },
        ];
        let fh = build_file_highlight(&[line0.clone(), line1.clone()]);
        assert_eq!(fh.tokens_for_line(0), line0);
        assert_eq!(fh.tokens_for_line(1), line1);
        assert_eq!(fh.tokens_for_line(2), vec![]); // out of bounds
    }
}
```

**Step 3: Run test to verify it passes**

Run: `cargo test -p ferret-indexer-core -- highlight`
Expected: 4 tests PASS

**Step 4: Register the module**

In `ferret-core/src/lib.rs`, add:
```rust
pub mod highlight;
```
and public exports:
```rust
pub use highlight::{
    FileHighlight, Token, TokenKind, build_file_highlight, decode_rle, encode_rle,
};
```

**Step 5: Commit**

```bash
git add ferret-core/Cargo.toml ferret-core/src/highlight.rs ferret-core/src/lib.rs
git commit -m "feat: add highlight module with TokenKind, RLE encoding, and FileHighlight"
```

---

### Task 2: Add HighlightStoreWriter and HighlightStoreReader

**Files:**
- Modify: `ferret-core/src/highlight.rs` (append writer/reader)
- Test: inline tests in `highlight.rs`

This follows the same pattern as `ContentStoreWriter`/`ContentStoreReader` in `content.rs`:
- Writer appends independently-compressed zstd blocks to a flat file
- Returns `(offset, compressed_len)` per file — stored in metadata
- Reader memory-maps the file and decompresses on demand

**Step 1: Write failing test for HighlightStoreWriter/Reader roundtrip**

Append to `highlight.rs` tests:

```rust
#[test]
fn test_highlight_store_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("highlights.zst");

    // Build highlight data for two files
    let fh0 = build_file_highlight(&[
        vec![Token { len: 2, kind: TokenKind::Keyword }, Token { len: 5, kind: TokenKind::Plain }],
        vec![Token { len: 10, kind: TokenKind::String }],
    ]);
    let fh1 = build_file_highlight(&[
        vec![Token { len: 3, kind: TokenKind::Comment }],
    ]);

    // Write
    let mut writer = HighlightStoreWriter::new(&path).unwrap();
    let (off0, len0, lines0) = writer.add_file(&fh0).unwrap();
    let (off1, len1, lines1) = writer.add_file(&fh1).unwrap();
    writer.finish().unwrap();

    assert_eq!(lines0, 2);
    assert_eq!(lines1, 1);

    // Read
    let reader = HighlightStoreReader::open(&path).unwrap();
    let read0 = reader.read_file(off0, len0, lines0).unwrap();
    let read1 = reader.read_file(off1, len1, lines1).unwrap();

    assert_eq!(read0.tokens_for_line(0), fh0.tokens_for_line(0));
    assert_eq!(read0.tokens_for_line(1), fh0.tokens_for_line(1));
    assert_eq!(read1.tokens_for_line(0), fh1.tokens_for_line(0));
}
```

**Step 2: Run test, confirm it fails (types don't exist yet)**

Run: `cargo test -p ferret-indexer-core -- test_highlight_store_roundtrip`
Expected: FAIL (cannot find `HighlightStoreWriter`)

**Step 3: Implement HighlightStoreWriter and HighlightStoreReader**

Append to `highlight.rs` (before `#[cfg(test)]`):

```rust
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use memmap2::Mmap;

/// Serialization format for a single file's highlights block (before zstd):
///
/// ```text
/// [line_count: u32 LE]
/// [line_offsets: u32 LE × line_count]   — byte offset into token_data for each line
/// [token_data: u8...]                    — flat RLE tokens
/// ```
fn serialize_file_highlight(fh: &FileHighlight) -> Vec<u8> {
    let line_count = fh.line_offsets.len() as u32;
    // Header: line_count + offsets + token_data
    let header_size = 4 + fh.line_offsets.len() * 4;
    let mut buf = Vec::with_capacity(header_size + fh.token_data.len());
    buf.extend_from_slice(&line_count.to_le_bytes());
    for &off in &fh.line_offsets {
        buf.extend_from_slice(&off.to_le_bytes());
    }
    buf.extend_from_slice(&fh.token_data);
    buf
}

fn deserialize_file_highlight(data: &[u8]) -> Option<FileHighlight> {
    if data.len() < 4 {
        return None;
    }
    let line_count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let offsets_end = 4 + line_count * 4;
    if data.len() < offsets_end {
        return None;
    }
    let mut line_offsets = Vec::with_capacity(line_count);
    for i in 0..line_count {
        let base = 4 + i * 4;
        let off = u32::from_le_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]);
        line_offsets.push(off);
    }
    let token_data = data[offsets_end..].to_vec();
    Some(FileHighlight {
        token_data,
        line_offsets,
    })
}

/// Writer for the per-segment `highlights.zst` file.
///
/// Mirrors `ContentStoreWriter`: appends independently zstd-compressed blocks,
/// returns `(offset, compressed_len, line_count)` per file for metadata storage.
pub struct HighlightStoreWriter {
    writer: BufWriter<File>,
    current_offset: u64,
}

impl HighlightStoreWriter {
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let file = File::create(path)?;
        Ok(Self {
            writer: BufWriter::new(file),
            current_offset: 0,
        })
    }

    /// Add a file's highlight data. Returns `(offset, compressed_len, line_count)`.
    pub fn add_file(&mut self, fh: &FileHighlight) -> std::io::Result<(u64, u32, u32)> {
        let serialized = serialize_file_highlight(fh);
        let compressed = zstd::bulk::compress(&serialized, 3)
            .map_err(std::io::Error::other)?;

        let offset = self.current_offset;
        let compressed_len: u32 = compressed.len().try_into().map_err(|_| {
            std::io::Error::other("compressed highlight block exceeds u32::MAX")
        })?;
        let line_count = fh.line_offsets.len() as u32;

        self.writer.write_all(&compressed)?;
        self.current_offset += compressed_len as u64;

        Ok((offset, compressed_len, line_count))
    }

    /// Add pre-compressed highlight data (for compaction copy-through).
    pub fn add_raw(&mut self, compressed: &[u8]) -> std::io::Result<(u64, u32)> {
        let offset = self.current_offset;
        let compressed_len: u32 = compressed.len().try_into().map_err(|_| {
            std::io::Error::other("compressed highlight block exceeds u32::MAX")
        })?;
        self.writer.write_all(compressed)?;
        self.current_offset += compressed_len as u64;
        Ok((offset, compressed_len))
    }

    pub fn finish(mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

/// Reader for the per-segment `highlights.zst` file.
pub struct HighlightStoreReader {
    mmap: Mmap,
}

impl HighlightStoreReader {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(Self { mmap })
    }

    /// Read a file's highlight data given its offset, compressed length, and line count.
    pub fn read_file(&self, offset: u64, compressed_len: u32, line_count: u32) -> Result<FileHighlight, crate::IndexError> {
        let start = offset as usize;
        let end = start + compressed_len as usize;
        if end > self.mmap.len() {
            return Err(crate::IndexError::IndexCorruption(
                "highlight block out of bounds".to_string(),
            ));
        }
        let compressed = &self.mmap[start..end];
        let decompressed = zstd::bulk::decompress(compressed, 10 * 1024 * 1024)
            .map_err(|e| crate::IndexError::IndexCorruption(format!("highlight zstd: {e}")))?;
        deserialize_file_highlight(&decompressed).ok_or_else(|| {
            crate::IndexError::IndexCorruption("malformed highlight block".to_string())
        })
    }

    /// Read raw compressed bytes for a file (for compaction copy-through).
    pub fn read_raw(&self, offset: u64, compressed_len: u32) -> Result<Vec<u8>, crate::IndexError> {
        let start = offset as usize;
        let end = start + compressed_len as usize;
        if end > self.mmap.len() {
            return Err(crate::IndexError::IndexCorruption(
                "highlight block out of bounds".to_string(),
            ));
        }
        Ok(self.mmap[start..end].to_vec())
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p ferret-indexer-core -- highlight`
Expected: all tests PASS

**Step 5: Add public exports to lib.rs**

```rust
pub use highlight::{
    FileHighlight, HighlightStoreReader, HighlightStoreWriter, Token, TokenKind,
    build_file_highlight, decode_rle, encode_rle,
};
```

**Step 6: Commit**

```bash
git add ferret-core/src/highlight.rs ferret-core/src/lib.rs
git commit -m "feat: add HighlightStoreWriter/Reader with zstd-compressed per-file blocks"
```

---

### Task 3: Add syntect tokenizer — `tokenize_file()` function

**Files:**
- Modify: `ferret-core/src/highlight.rs` (add tokenization)
- Test: inline tests

**Step 1: Write failing test for tokenize_file**

```rust
#[test]
fn test_tokenize_rust_file() {
    let code = b"fn main() {\n    let x = 42;\n}\n";
    let tokens = tokenize_file(code, Language::Rust).unwrap();
    // Should have 3 lines of tokens
    assert_eq!(tokens.len(), 3);
    // First line should start with a Keyword token for "fn"
    assert_eq!(tokens[0][0].kind, TokenKind::Keyword);
    assert_eq!(tokens[0][0].len, 2); // "fn"
}

#[test]
fn test_tokenize_unknown_language_returns_none() {
    let code = b"hello world\n";
    assert!(tokenize_file(code, Language::Unknown).is_none());
}
```

**Step 2: Run test, confirm it fails**

**Step 3: Implement tokenize_file**

```rust
use syntect::parsing::{SyntaxSet, ParseState, ScopeStack};
use crate::types::Language;
use std::sync::LazyLock;

/// Shared syntect SyntaxSet — loaded once, reused across all tokenization calls.
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

/// Map our Language enum to syntect file extensions.
fn language_to_syntect_ext(lang: Language) -> Option<&'static str> {
    match lang {
        Language::Rust => Some("rs"),
        Language::Python => Some("py"),
        Language::JavaScript => Some("js"),
        Language::TypeScript => Some("ts"),
        Language::Go => Some("go"),
        Language::C => Some("c"),
        Language::Cpp => Some("cpp"),
        Language::Java => Some("java"),
        Language::Ruby => Some("rb"),
        Language::Swift => Some("swift"),
        Language::Kotlin => Some("kt"),
        Language::Scala => Some("scala"),
        Language::Haskell => Some("hs"),
        Language::Lua => Some("lua"),
        Language::Perl => Some("pl"),
        Language::Php => Some("php"),
        Language::CSharp => Some("cs"),
        Language::Shell => Some("sh"),
        Language::Html => Some("html"),
        Language::Css => Some("css"),
        Language::Sql => Some("sql"),
        Language::Json => Some("json"),
        Language::Xml => Some("xml"),
        Language::Yaml => Some("yaml"),
        Language::Toml => Some("toml"),
        Language::Markdown => Some("md"),
        Language::Dockerfile => Some("dockerfile"),
        Language::Makefile => Some("makefile"),
        Language::Protobuf => Some("proto"),
        Language::Tsx => Some("tsx"),
        Language::ObjectiveC => Some("m"),
        Language::Dart => Some("dart"),
        Language::Elixir => Some("ex"),
        Language::Erlang => Some("erl"),
        Language::Clojure => Some("clj"),
        Language::R => Some("r"),
        Language::Matlab => Some("matlab"),
        Language::Groovy => Some("groovy"),
        Language::Zig => Some("zig"),
        Language::Ocaml => Some("ml"),
        Language::Lisp => Some("lisp"),
        _ => None,
    }
}

/// Classify a syntect scope stack into our 16-category TokenKind.
fn classify_scope(stack: &ScopeStack) -> TokenKind {
    for scope in stack.as_slice().iter().rev() {
        let s = format!("{scope}");
        if s.starts_with("comment") { return TokenKind::Comment; }
        if s.starts_with("string") { return TokenKind::String; }
        if s.starts_with("constant.numeric") { return TokenKind::Number; }
        if s.starts_with("constant") { return TokenKind::Constant; }
        if s.starts_with("keyword") { return TokenKind::Keyword; }
        if s.starts_with("storage.type") { return TokenKind::Type; }
        if s.starts_with("storage") { return TokenKind::Keyword; }
        if s.starts_with("entity.name.function") { return TokenKind::Function; }
        if s.starts_with("entity.name.type") { return TokenKind::Type; }
        if s.starts_with("entity.name.macro") { return TokenKind::Macro; }
        if s.starts_with("entity.name") { return TokenKind::Function; }
        if s.starts_with("variable") { return TokenKind::Variable; }
        if s.starts_with("support.function") { return TokenKind::Function; }
        if s.starts_with("support.type") { return TokenKind::Type; }
        if s.starts_with("support.macro") { return TokenKind::Macro; }
        if s.starts_with("punctuation") { return TokenKind::Punctuation; }
        if s.starts_with("meta.attribute") { return TokenKind::Attribute; }
    }
    TokenKind::Plain
}

/// Tokenize file content into per-line token lists using syntect.
///
/// Returns `Some(line_tokens)` if the language is recognized by syntect,
/// or `None` if unsupported. Callers should skip highlight storage for `None`.
pub fn tokenize_file(content: &[u8], language: Language) -> Option<Vec<Vec<Token>>> {
    let ext = language_to_syntect_ext(language)?;
    let ss = &*SYNTAX_SET;
    let syntax = ss.find_syntax_by_extension(ext)?;
    let text = String::from_utf8_lossy(content);

    let mut parse_state = ParseState::new(syntax);
    let mut scope_stack = ScopeStack::new();
    let mut all_line_tokens = Vec::new();

    for line in syntect::util::LinesWithEndings::from(&text) {
        let ops = match parse_state.parse_line(line, ss) {
            Ok(ops) => ops,
            Err(_) => {
                // Parse error — emit entire line as Plain
                all_line_tokens.push(vec![Token {
                    len: line.len(),
                    kind: TokenKind::Plain,
                }]);
                continue;
            }
        };

        let mut line_tokens = Vec::new();
        let mut pos = 0;

        for (offset, op) in &ops {
            let offset = *offset;
            if offset > pos {
                let kind = classify_scope(&scope_stack);
                line_tokens.push(Token {
                    len: offset - pos,
                    kind,
                });
            }
            let _ = scope_stack.apply(op);
            pos = offset;
        }
        // Remaining text on line
        if pos < line.len() {
            let kind = classify_scope(&scope_stack);
            line_tokens.push(Token {
                len: line.len() - pos,
                kind,
            });
        }
        all_line_tokens.push(line_tokens);
    }

    Some(all_line_tokens)
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p ferret-indexer-core -- test_tokenize`
Expected: PASS

**Step 5: Commit**

```bash
git add ferret-core/src/highlight.rs
git commit -m "feat: add tokenize_file() using syntect for syntax classification"
```

---

### Task 4: Extend metadata to store highlight offsets

**Files:**
- Modify: `ferret-core/src/metadata.rs`
- Test: existing metadata tests + new test

The metadata entry is currently 58 bytes fixed. We need to add 3 fields per file:
- `highlight_offset: u64` — offset into `highlights.zst`
- `highlight_len: u32` — compressed length
- `highlight_lines: u32` — number of lines (needed to reconstruct FileHighlight)

This bumps the entry to 74 bytes and requires a version bump (1 → 2).

**Step 1: Write failing test**

```rust
#[test]
fn test_metadata_v2_highlight_fields_roundtrip() {
    // Build metadata with highlight fields set, write, read back, verify.
    let mut builder = MetadataBuilder::new();
    builder.add_file(FileMetadata {
        file_id: FileId(0),
        path: "test.rs".to_string(),
        content_hash: [0u8; 16],
        language: Language::Rust,
        size_bytes: 100,
        mtime_epoch_secs: 1700000000,
        line_count: 10,
        content_offset: 0,
        content_len: 50,
        highlight_offset: 1024,
        highlight_len: 200,
        highlight_lines: 10,
    });
    // Write to buffers, read back, assert highlight fields match
}
```

**Step 2: Add fields to FileMetadata**

In `metadata.rs`, add to `FileMetadata`:
```rust
pub highlight_offset: u64,
pub highlight_len: u32,
pub highlight_lines: u32,
```

**Step 3: Update the binary format**

- Bump `META_VERSION` from 1 to 2
- Update `ENTRY_SIZE` from 58 to 74 (adding 16 bytes: u64 + u32 + u32)
- Update `write_to()` to write the 3 new fields after `content_len`
- Update `MetadataReader::get()` to read the 3 new fields
- Update `MetadataReader::new()` to accept both version 1 (58-byte entries, highlight fields default to 0) and version 2 (74-byte entries) for backward compatibility
- Update `get_size_bytes()` similarly

**Step 4: Update all call sites that construct FileMetadata**

Every place that creates a `FileMetadata` needs the new fields. Initially set them all to `highlight_offset: 0, highlight_len: 0, highlight_lines: 0` — Task 5 will wire in real values.

Call sites (search for `FileMetadata {`):
- `ferret-core/src/segment.rs` — `build_inner()` line ~450 and `build_compact_inner()` line ~566
- `ferret-core/src/multi_search.rs` — any test fixtures
- `ferret-core/src/metadata.rs` — tests

**Step 5: Update recovery validation**

In `ferret-core/src/recovery.rs`, update `validate_meta_header()` to accept version 1 OR version 2.

**Step 6: Run all tests**

Run: `cargo test --workspace`
Expected: all PASS

**Step 7: Commit**

```bash
git add ferret-core/src/metadata.rs ferret-core/src/segment.rs ferret-core/src/recovery.rs ferret-core/src/multi_search.rs
git commit -m "feat: extend metadata with highlight_offset/len/lines fields (version 2)"
```

---

### Task 5: Generate highlights.zst during segment build

**Files:**
- Modify: `ferret-core/src/segment.rs` — `build_inner()` and `build_compact_inner()`
- Test: existing `test_segment_writer_creates_all_files` + new test

**Step 1: Write failing test**

```rust
#[test]
fn test_segment_writer_creates_highlights_file() {
    let dir = tempfile::tempdir().unwrap();
    let base_dir = dir.path().join(".ferret_index/segments");
    std::fs::create_dir_all(&base_dir).unwrap();

    let writer = SegmentWriter::new(base_dir.clone(), SegmentId(1));
    let files = vec![InputFile {
        path: "test.rs".to_string(),
        content: b"fn main() {\n    println!(\"hello\");\n}\n".to_vec(),
        mtime: 1700000000,
    }];
    let segment = writer.build(files).unwrap();

    // highlights.zst should exist
    assert!(base_dir.join("seg_0001/highlights.zst").exists());

    // Metadata should have non-zero highlight fields
    let meta = segment.metadata_reader().get(FileId(0)).unwrap();
    assert!(meta.highlight_len > 0);
    assert_eq!(meta.highlight_lines, 3); // 3 lines
}
```

**Step 2: Modify build_inner() Phase 1**

In the parallel phase (lines 404–434 of `segment.rs`), add to `ProcessedFile`:
```rust
highlight_data: Vec<u8>,        // serialized + compressed FileHighlight
highlight_lines: u32,
```

In the parallel `.map()`:
```rust
let highlight_compressed: Option<(Vec<u8>, u32)> =
    crate::highlight::tokenize_file(&input.content, language).map(|line_tokens| {
        let fh = crate::highlight::build_file_highlight(&line_tokens);
        let serialized = crate::highlight::serialize_file_highlight(&fh);
        let compressed = zstd::bulk::compress(&serialized, 3)
            .expect("zstd compress highlight");
        let lines = fh.line_offsets.len() as u32;
        (compressed, lines)
    });
```

Add to `ProcessedFile`:
```rust
highlight_compressed: Option<(Vec<u8>, u32)>, // (compressed_data, line_count) or None
```

Note: `serialize_file_highlight` is currently private. Make it `pub(crate)`.

**Step 3: Modify build_inner() Phase 2**

After creating `content_writer`, also create:
```rust
let mut highlight_writer =
    HighlightStoreWriter::new(&temp_dir.join("highlights.zst")).map_err(IndexError::Io)?;
```

In the sequential loop, after writing content:
```rust
let (highlight_offset, highlight_len, highlight_lines) =
    if let Some((ref compressed, lines)) = proc_file.highlight_compressed {
        let (off, len) = highlight_writer
            .add_raw(compressed)
            .map_err(IndexError::Io)?;
        (off, len, lines)
    } else {
        (0, 0, 0) // No highlights for this file
    };
```

And update the `FileMetadata` construction:
```rust
highlight_offset,
highlight_len,
highlight_lines,
```

**Step 4: Modify build_inner() Phase 3 — finalize**

After `content_writer.finish()`:
```rust
highlight_writer.finish().map_err(IndexError::Io)?;
```

**Step 5: Update Segment::open() to load HighlightStoreReader**

Add field to `Segment`:
```rust
highlight_reader: Option<HighlightStoreReader>,
```

In `Segment::open()`, after opening content reader:
```rust
let highlights_path = dir_path.join("highlights.zst");
let highlight_reader = if highlights_path.exists() {
    Some(HighlightStoreReader::open(&highlights_path).map_err(IndexError::Io)?)
} else {
    None
};
```

Add accessor:
```rust
pub fn highlight_reader(&self) -> Option<&HighlightStoreReader> {
    self.highlight_reader.as_ref()
}
```

**Step 6: Update build_compact_inner() for compaction**

In `build_compact_inner()`, add a `HighlightStoreWriter`. For compaction, we need to re-tokenize since file_ids are remapped. Follow the same pattern as symbol re-extraction:

```rust
let line_tokens = crate::highlight::tokenize_file(&file.raw_content, file.language);
let file_highlight = crate::highlight::build_file_highlight(&line_tokens);
```

Write to highlight store and set metadata fields.

**Step 7: Add `highlight_compressed` to CompactInputFile (optimization for future)**

For now, compaction re-tokenizes (matching how symbols re-extract). This keeps the initial implementation simple. A future optimization could carry forward raw compressed highlight blocks during compaction via `HighlightStoreReader::read_raw()` + `HighlightStoreWriter::add_raw()`, but since file_ids don't affect highlights (unlike trigrams), re-tokenization is correct and simple.

For files where `tokenize_file()` returns `None`, set metadata highlight fields to `(0, 0, 0)` — no entry written to `highlights.zst`.

**Step 8: Run tests**

Run: `cargo test --workspace`
Expected: all PASS

**Step 9: Run clippy and fmt**

```bash
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

**Step 10: Commit**

```bash
git add ferret-core/src/segment.rs ferret-core/src/highlight.rs
git commit -m "feat: generate highlights.zst during segment build and compaction"
```

---

### Task 6: Wire highlights through search results

**Files:**
- Modify: `ferret-core/src/search.rs` — add highlight tokens to LineMatch
- Modify: `ferret-core/src/multi_search.rs` — populate highlights from segment reader
- Test: integration test

This task makes highlights *available* in search results. The web UI rendering (consuming these tokens) is out of scope for this plan.

**Step 1: Add highlight_tokens field to LineMatch and ContextLine**

In `search.rs`:
```rust
pub struct LineMatch {
    pub line_number: u32,
    pub content: String,
    pub ranges: Vec<(usize, usize)>,
    pub context_before: Vec<ContextLine>,
    pub context_after: Vec<ContextLine>,
    /// Syntax highlight tokens for this line (empty if highlights unavailable).
    pub highlight_tokens: Vec<crate::highlight::Token>,
}

pub struct ContextLine {
    pub line_number: u32,
    pub content: String,
    /// Syntax highlight tokens for this context line.
    pub highlight_tokens: Vec<crate::highlight::Token>,
}
```

**Step 2: Update all LineMatch/ContextLine construction sites**

Add `highlight_tokens: vec![]` to every construction site (search for `LineMatch {` and `ContextLine {` in multi_search.rs and verify.rs). This keeps existing behavior unchanged — highlights are empty by default.

**Step 3: Populate highlights in multi_search.rs**

In the search result building code, after constructing `LineMatch`es for a file, if the segment has a `highlight_reader`, load the `FileHighlight` and attach tokens to each `LineMatch` and `ContextLine` by line number:

```rust
// After building line_matches for a file — only if highlights were stored
if meta.highlight_len > 0 {
    if let Some(hr) = segment.highlight_reader() {
        if let Ok(fh) = hr.read_file(meta.highlight_offset, meta.highlight_len, meta.highlight_lines) {
            for lm in &mut line_matches {
                lm.highlight_tokens = fh.tokens_for_line((lm.line_number - 1) as usize);
                for ctx in &mut lm.context_before {
                    ctx.highlight_tokens = fh.tokens_for_line((ctx.line_number - 1) as usize);
                }
                for ctx in &mut lm.context_after {
                    ctx.highlight_tokens = fh.tokens_for_line((ctx.line_number - 1) as usize);
                }
            }
        }
    }
}
```

Files with `highlight_len == 0` (unsupported languages) get empty `highlight_tokens` vecs — the web UI should fall back to unhighlighted rendering.

**Step 4: Update serde for LineMatch**

`FileMatch` derives `Serialize, Deserialize, Clone` — the new `Token` type needs these derives too. Add to `Token` and `TokenKind`:
```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
```

**Step 5: Run all tests**

Run: `cargo test --workspace`
Expected: all PASS

**Step 6: Run clippy and fmt**

```bash
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

**Step 7: Commit**

```bash
git add ferret-core/src/search.rs ferret-core/src/multi_search.rs ferret-core/src/highlight.rs ferret-core/src/verify.rs
git commit -m "feat: wire highlight tokens through search results (LineMatch/ContextLine)"
```

---

### Task 7: Expose highlights in file preview (daemon GetFile response)

**Files:**
- Modify: `ferret-daemon/src/types.rs` or `json_protocol.rs` — add highlight tokens to FileResponse
- Modify: `ferret-cli/src/daemon.rs` — populate highlights when serving GetFile requests
- Test: manual (daemon integration)

**Step 1: Add highlight_tokens to FileResponse lines**

In the daemon's `FileResponse`, each line is currently a `String`. Change to include optional token data. The simplest approach: add a parallel `Vec<Vec<Token>>` field:

```rust
pub struct FileResponse {
    pub path: String,
    pub language: String,
    pub total_lines: usize,
    pub lines: Vec<String>,
    /// Per-line syntax tokens (same length as `lines`, empty vecs if unavailable).
    pub highlight_tokens: Vec<Vec<Token>>,
}
```

**Step 2: Populate in daemon GetFile handler**

When the daemon handles a `GetFile` request, it already reads the file content from the segment. After building the lines, also load the `FileHighlight` from the segment's highlight reader and extract tokens per line.

**Step 3: Run all tests, clippy, fmt**

**Step 4: Commit**

```bash
git add ferret-daemon/ ferret-cli/src/daemon.rs
git commit -m "feat: include highlight tokens in daemon GetFile responses"
```

---

### Task 8: Clean up benchmark files and update CLAUDE.md

**Files:**
- Delete: `ferret-core/examples/bench_syntect.rs`
- Delete: `ferret-core/examples/bench_syntect_storage.rs`
- Modify: `ferret-core/Cargo.toml` — move syntect from dev-dependencies to dependencies (if not done in Task 1)
- Modify: `CLAUDE.md` — document highlights.zst in the segment layout and binary format sections

**Step 1: Remove benchmark examples**

```bash
rm ferret-core/examples/bench_syntect.rs ferret-core/examples/bench_syntect_storage.rs
```

**Step 2: Update CLAUDE.md**

Add to the on-disk segment layout:
```
    highlights.zst  # Pre-computed syntax highlight tokens (optional)
```

Add binary format documentation:
```
**highlights.zst:**
Per-file blocks, independently zstd-compressed. Each decompressed block:
  [line_count: u32 LE]
  [line_offsets: u32 LE × line_count]  — byte offset into token_data per line
  [token_data: u8...]                   — RLE tokens: (len:u16 LE, kind:u8) triples

Token kinds (4-bit enum, stored as u8):
  0=Plain, 1=Keyword, 2=String, 3=Comment, 4=Number, 5=Function,
  6=Type, 7=Variable, 8=Operator, 9=Punctuation, 10=Macro,
  11=Attribute, 12=Constant, 13=Module, 14=Label, 15=Other

Location metadata per file stored in meta.bin v2:
  highlight_offset:u64 | highlight_len:u32 | highlight_lines:u32
```

**Step 3: Run full CI check**

```bash
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
cargo test --workspace
```

**Step 4: Commit**

```bash
git add -A
git commit -m "chore: clean up benchmarks and document highlights.zst format"
```
