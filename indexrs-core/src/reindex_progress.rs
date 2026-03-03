//! Structured progress events emitted during reindex operations.

use serde::{Deserialize, Serialize};

/// A structured progress event emitted during reindex.
///
/// Sent as JSON over the daemon wire protocol inside
/// `DaemonResponse::Progress { message }`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReindexProgress {
    /// Change detection started.
    DetectingChanges,
    /// Fell back to hash-based scanning (git unavailable).
    ScanningFallback,
    /// Change detection complete.
    ChangesDetected {
        created: usize,
        modified: usize,
        deleted: usize,
    },
    /// No changes found.
    NoChanges,
    /// Waiting for the write lock (another operation is in progress).
    WaitingForLock,
    /// Reading and filtering changed files before indexing.
    PreparingFiles { current: usize, total: usize },
    /// Building a segment: file `files_done` of `files_total` processed.
    BuildingSegment {
        segment_id: u32,
        files_done: usize,
        files_total: usize,
    },
    /// Writing tombstones for old file entries.
    Tombstoning { count: u32 },
    /// Segment compaction started.
    CompactingSegments { input_segments: usize },
    /// Reindex finished successfully.
    Complete { changes_applied: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serde_roundtrip_detecting_changes() {
        let event = ReindexProgress::DetectingChanges;
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, r#"{"type":"detecting_changes"}"#);
        let back: ReindexProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn test_serde_roundtrip_building_segment() {
        let event = ReindexProgress::BuildingSegment {
            segment_id: 3,
            files_done: 100,
            files_total: 500,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ReindexProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn test_serde_roundtrip_changes_detected() {
        let event = ReindexProgress::ChangesDetected {
            created: 10,
            modified: 20,
            deleted: 5,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ReindexProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn test_serde_roundtrip_complete() {
        let event = ReindexProgress::Complete {
            changes_applied: 42,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ReindexProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }
}
