pub mod error;
pub mod search;
pub mod types;

pub use error::{IndexError, Result};
pub use search::{FileMatch, LineMatch, SearchResult};
pub use types::{FileId, Language, SegmentId, SymbolKind, Trigram};
