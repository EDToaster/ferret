pub mod codec;
pub mod content;
pub mod error;
pub mod metadata;
pub mod posting;
pub mod search;
pub mod trigram;
pub mod types;

pub use codec::{
    decode_delta_varint, decode_positional_postings, encode_delta_varint,
    encode_positional_postings,
};
pub use content::{ContentStoreReader, ContentStoreWriter};
pub use error::{IndexError, Result};
pub use metadata::{FileMetadata, MetadataBuilder, MetadataReader};
pub use posting::PostingListBuilder;
pub use search::{FileMatch, LineMatch, SearchResult};
pub use trigram::{extract_trigrams, extract_unique_trigrams};
pub use types::{FileId, Language, SegmentId, SymbolKind, Trigram};
