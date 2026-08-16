//! Cold storage for activity segments aged out of the 24-hour hot window.
//!
//! Chunks are fixed 30-day epoch blocks, not calendar months: this crate has
//! no date library and must not gain one. A segment belongs to the chunk of
//! its `start_ms`, even when it ends in the next block, so chunk assignment
//! never truncates a segment.
//!
//! Failure posture mirrors the rest of the crate: a missing, unreadable, or
//! malformed chunk is skipped rather than panicking, and (unlike the 24-hour
//! hot loader) a single bad segment inside an otherwise-good chunk is skipped
//! individually rather than discarding the chunk or the whole range — across
//! 90 days of archives, dropping everything for one bad timestamp would erase
//! months of history.
//!
//! `archive_segments` and `prune_chunks` are wired into the hot-file prune in
//! `activity.rs`. `read_range` is read through by the `get_activity_range`
//! command, also in `activity.rs`, which merges it with the hot set.

use crate::activity::{
    persist_history, PersistedActivityHistory, PersistedKind, PersistedSegment, Segment,
    HISTORY_SCHEMA_VERSION,
};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

#[cfg(test)]
use crate::activity::ActivityKind;

/// Archive chunk width. Fixed epoch blocks, deliberately not calendar months:
/// the crate has no date library and must not gain one.
pub(crate) const ARCHIVE_BLOCK_MS: u64 = 30 * 24 * 60 * 60 * 1_000;

const CHUNK_FILE_PREFIX: &str = "activity-archive-";
const CHUNK_FILE_SUFFIX: &str = ".json";

/// Chunk a segment belongs to, keyed by its start.
pub(crate) fn chunk_key(start_ms: u64) -> u64 {
    start_ms / ARCHIVE_BLOCK_MS
}

/// Absolute path of one chunk inside the config directory.
pub(crate) fn chunk_path(config_dir: &Path, key: u64) -> PathBuf {
    config_dir.join(format!("{CHUNK_FILE_PREFIX}{key}{CHUNK_FILE_SUFFIX}"))
}

/// Parse a chunk file's segments, dropping any individually invalid entry
/// (`end_ms < start_ms`). Returns an empty vector for a missing, unreadable,
/// malformed, or schema-mismatched chunk rather than failing — callers decide
/// what "empty" means for their situation (a fresh chunk on write, a gap in
/// coverage on read).
fn read_chunk_segments(path: &Path) -> Vec<Segment> {
    let Ok(contents) = fs::read(path) else {
        return Vec::new();
    };
    let Ok(history) = serde_json::from_slice::<PersistedActivityHistory>(&contents) else {
        return Vec::new();
    };
    if history.version != HISTORY_SCHEMA_VERSION {
        return Vec::new();
    }
    history
        .segments
        .into_iter()
        .filter(|item| item.end_ms >= item.start_ms)
        .map(|item| Segment {
            kind: item.kind.into_activity(),
            start_ms: item.start_ms,
            end_ms: item.end_ms,
        })
        .collect()
}

fn persisted_from_segments(segments: &[Segment]) -> PersistedActivityHistory {
    let segments = segments
        .iter()
        .filter_map(|segment| {
            let kind = PersistedKind::from_activity(segment.kind)?;
            Some(PersistedSegment {
                kind,
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
            })
        })
        .collect();
    PersistedActivityHistory {
        version: HISTORY_SCHEMA_VERSION,
        segments,
    }
}

/// Merge segments into their chunks. Returns Err if any write failed; the
/// caller must then keep those segments hot and retry later.
///
/// Merging is idempotent: re-archiving a segment already present in its chunk
/// (as happens on a retry after a partial failure) does not duplicate it.
pub(crate) fn archive_segments(config_dir: &Path, segments: &[Segment]) -> io::Result<()> {
    let mut by_key: BTreeMap<u64, Vec<Segment>> = BTreeMap::new();
    for &segment in segments {
        by_key
            .entry(chunk_key(segment.start_ms))
            .or_default()
            .push(segment);
    }

    for (key, new_segments) in by_key {
        let path = chunk_path(config_dir, key);
        let mut merged = read_chunk_segments(&path);
        merged.extend(new_segments);
        merged.sort_by_key(|segment| segment.start_ms);
        merged.dedup();
        persist_history(&path, &persisted_from_segments(&merged))?;
    }
    Ok(())
}

/// Every archived segment overlapping `[start_ms, end_ms)`, oldest first.
/// Skips unreadable or corrupt chunks rather than failing the whole read.
///
/// Read through by the `get_activity_range` command (`activity.rs`), merged
/// there with the hot set.
pub(crate) fn read_range(config_dir: &Path, start_ms: u64, end_ms: u64) -> Vec<Segment> {
    if end_ms <= start_ms {
        return Vec::new();
    }
    let end_key = chunk_key(end_ms.saturating_sub(1));

    let mut results = Vec::new();
    let Ok(entries) = fs::read_dir(config_dir) else {
        return results;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(key) = file_name.to_str().and_then(parse_chunk_key) else {
            continue;
        };
        if key <= end_key {
            results.extend(read_chunk_segments(&entry.path()));
        }
    }

    // Keep every segment that overlaps the requested window, straddlers at
    // both ends included whole. `read_chunk_segments` already dropped the
    // one malformed shape (`end_ms < start_ms`); there is no further
    // per-segment skip here. In particular, a segment is never treated as a
    // "future" clock-skew signal relative to `end_ms`: every historical
    // query has an `end_ms` in the past, so a real segment routinely ends
    // after it (that is exactly the end-boundary straddler), and a stray
    // future timestamp can never overlap a sane range anyway. Clamping to
    // the window, like `summary`'s `start_ms.max(window_start)`, is the
    // caller's job, not this function's.
    results.retain(|segment| segment.end_ms > start_ms && segment.start_ms < end_ms);
    results.sort_by_key(|segment| segment.start_ms);
    results.dedup();
    results
}

/// Delete chunks whose entire block lies before `cutoff_ms`.
///
/// A failure to delete one chunk does not stop the others; the last error
/// encountered, if any, is returned after every chunk has been attempted.
pub(crate) fn prune_chunks(config_dir: &Path, cutoff_ms: u64) -> io::Result<()> {
    let entries = match fs::read_dir(config_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    let mut last_error = None;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(key) = parse_chunk_key(name) else {
            continue;
        };
        if key.saturating_add(1).saturating_mul(ARCHIVE_BLOCK_MS) <= cutoff_ms
            && read_chunk_segments(&entry.path())
                .iter()
                .all(|segment| segment.end_ms <= cutoff_ms)
        {
            if let Err(error) = fs::remove_file(entry.path()) {
                if error.kind() != io::ErrorKind::NotFound {
                    last_error = Some(error);
                }
            }
        }
    }

    match last_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn parse_chunk_key(file_name: &str) -> Option<u64> {
    file_name
        .strip_prefix(CHUNK_FILE_PREFIX)?
        .strip_suffix(CHUNK_FILE_SUFFIX)?
        .parse::<u64>()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..100 {
                let id = TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "unfocus-activity-archive-tests-{}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("test directory should be created: {error}"),
                }
            }
            panic!("could not allocate a test activity archive directory")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn chunk_key_is_block_division() {
        assert_eq!(chunk_key(0), 0);
        assert_eq!(chunk_key(ARCHIVE_BLOCK_MS - 1), 0);
        assert_eq!(chunk_key(ARCHIVE_BLOCK_MS), 1);
        assert_eq!(chunk_key(ARCHIVE_BLOCK_MS * 5 + 100), 5);
    }

    #[test]
    fn chunk_path_names_file_by_key() {
        let dir = PathBuf::from("/config/dir");
        assert_eq!(chunk_path(&dir, 7), dir.join("activity-archive-7.json"));
    }

    #[test]
    fn archive_segments_groups_by_start_chunk() {
        let dir = TestDirectory::new();
        let seg_same_block = Segment {
            kind: ActivityKind::Active,
            start_ms: 1_000,
            end_ms: 2_000,
        };
        let seg_next_block = Segment {
            kind: ActivityKind::Afk,
            start_ms: ARCHIVE_BLOCK_MS + 500,
            end_ms: ARCHIVE_BLOCK_MS + 900,
        };
        // Starts in block 0 but ends in block 1: must stay keyed by its start.
        let seg_straddles = Segment {
            kind: ActivityKind::Active,
            start_ms: ARCHIVE_BLOCK_MS - 100,
            end_ms: ARCHIVE_BLOCK_MS + 50,
        };

        archive_segments(&dir.path, &[seg_same_block, seg_next_block, seg_straddles])
            .expect("archive");

        let chunk0 = read_chunk_segments(&chunk_path(&dir.path, 0));
        let chunk1 = read_chunk_segments(&chunk_path(&dir.path, 1));

        assert!(chunk0.contains(&seg_same_block));
        assert!(
            chunk0.contains(&seg_straddles),
            "segment keyed by its start stays in chunk 0 even though it ends in block 1"
        );
        assert!(!chunk1.contains(&seg_straddles));
        assert_eq!(chunk0.len(), 2);
        assert_eq!(chunk1, vec![seg_next_block]);
    }

    #[test]
    fn archive_segments_merges_into_existing_chunk() {
        let dir = TestDirectory::new();
        let first = Segment {
            kind: ActivityKind::Active,
            start_ms: 1_000,
            end_ms: 2_000,
        };
        archive_segments(&dir.path, &[first]).expect("first archive");

        let second = Segment {
            kind: ActivityKind::Afk,
            start_ms: 3_000,
            end_ms: 4_000,
        };
        archive_segments(&dir.path, &[second]).expect("second archive");

        assert_eq!(
            read_chunk_segments(&chunk_path(&dir.path, 0)),
            vec![first, second]
        );

        // A third write arriving earlier than both must merge in sorted order,
        // not simply append.
        let third = Segment {
            kind: ActivityKind::Active,
            start_ms: 500,
            end_ms: 900,
        };
        archive_segments(&dir.path, &[third]).expect("third archive");
        assert_eq!(
            read_chunk_segments(&chunk_path(&dir.path, 0)),
            vec![third, first, second]
        );
    }

    #[test]
    fn read_range_includes_preceding_chunk_for_straddler() {
        let dir = TestDirectory::new();
        let straddler = Segment {
            kind: ActivityKind::Active,
            start_ms: ARCHIVE_BLOCK_MS - 100,
            end_ms: ARCHIVE_BLOCK_MS + 50,
        };
        archive_segments(&dir.path, &[straddler]).expect("archive");

        // Query window is entirely inside block 1; the straddler lives in
        // chunk 0 (keyed by its start) but overlaps this window and must
        // still be found.
        let results = read_range(&dir.path, ARCHIVE_BLOCK_MS, ARCHIVE_BLOCK_MS + 1_000);
        assert_eq!(results, vec![straddler]);
    }

    #[test]
    fn read_range_finds_segment_spanning_multiple_preceding_chunks() {
        let dir = TestDirectory::new();
        let straddler = Segment {
            kind: ActivityKind::Active,
            start_ms: ARCHIVE_BLOCK_MS - 100,
            end_ms: ARCHIVE_BLOCK_MS * 3 + 50,
        };
        archive_segments(&dir.path, &[straddler]).expect("archive");

        let results = read_range(
            &dir.path,
            ARCHIVE_BLOCK_MS * 3,
            ARCHIVE_BLOCK_MS * 3 + 1_000,
        );
        assert_eq!(results, vec![straddler]);
    }

    #[test]
    fn read_range_returns_each_segment_once() {
        let dir = TestDirectory::new();
        let segment = Segment {
            kind: ActivityKind::Active,
            start_ms: 10_000,
            end_ms: 20_000,
        };
        archive_segments(&dir.path, &[segment]).expect("archive");

        // start_ms is 0, so chunk_key(0) == 0 and there is no preceding chunk
        // to read: this pins that chunk 0 is not read twice.
        let results = read_range(&dir.path, 0, ARCHIVE_BLOCK_MS);
        assert_eq!(results, vec![segment]);
    }

    #[test]
    fn read_range_skips_corrupt_chunk_and_reads_the_rest() {
        let dir = TestDirectory::new();
        let good = Segment {
            kind: ActivityKind::Active,
            start_ms: ARCHIVE_BLOCK_MS + 1_000,
            end_ms: ARCHIVE_BLOCK_MS + 2_000,
        };
        archive_segments(&dir.path, &[good]).expect("archive good chunk");
        fs::write(chunk_path(&dir.path, 0), b"{not-json").expect("corrupt chunk 0");

        let results = read_range(&dir.path, 0, ARCHIVE_BLOCK_MS * 2);
        assert_eq!(results, vec![good]);
    }

    #[test]
    fn read_range_skips_one_bad_segment_not_the_chunk() {
        let dir = TestDirectory::new();
        let key = 3;
        let block_start = key * ARCHIVE_BLOCK_MS;
        let good = PersistedSegment {
            kind: PersistedKind::Active,
            start_ms: block_start + 1_000,
            end_ms: block_start + 2_000,
        };
        let good_start = good.start_ms;
        let good_end = good.end_ms;
        // end_ms < start_ms: the sole malformed shape, skipped individually.
        let bad = PersistedSegment {
            kind: PersistedKind::Afk,
            start_ms: block_start + 5_000,
            end_ms: block_start + 1_000,
        };
        persist_history(
            &chunk_path(&dir.path, key),
            &PersistedActivityHistory {
                version: HISTORY_SCHEMA_VERSION,
                segments: vec![good, bad],
            },
        )
        .expect("seed chunk");

        let results = read_range(&dir.path, block_start, block_start + ARCHIVE_BLOCK_MS);
        assert_eq!(results.len(), 1, "the whole chunk must not be discarded");
        assert_eq!(results[0].start_ms, good_start);
        assert_eq!(results[0].end_ms, good_end);
    }

    #[test]
    fn read_range_includes_segment_straddling_the_range_end() {
        let dir = TestDirectory::new();
        // Starts inside the query range but ends after it: this is the
        // week-view case (the last day's final block) that a future-relative-
        // to-`end_ms` rule would wrongly discard. It must come back whole,
        // not clamped or dropped.
        let straddler = Segment {
            kind: ActivityKind::Active,
            start_ms: 5_000,
            end_ms: ARCHIVE_BLOCK_MS + 500,
        };
        archive_segments(&dir.path, &[straddler]).expect("archive");

        let results = read_range(&dir.path, 0, ARCHIVE_BLOCK_MS);
        assert_eq!(results, vec![straddler]);
    }

    #[test]
    fn prune_chunks_deletes_only_fully_expired_blocks() {
        let dir = TestDirectory::new();
        let seg0 = Segment {
            kind: ActivityKind::Active,
            start_ms: 500,
            end_ms: 900,
        };
        let seg1 = Segment {
            kind: ActivityKind::Afk,
            start_ms: ARCHIVE_BLOCK_MS + 500,
            end_ms: ARCHIVE_BLOCK_MS + 900,
        };
        archive_segments(&dir.path, &[seg0, seg1]).expect("archive");

        // Cutoff lands inside block 1: block 0's entire span is expired,
        // block 1's is not.
        let cutoff = ARCHIVE_BLOCK_MS + 100;
        prune_chunks(&dir.path, cutoff).expect("prune");

        assert!(!chunk_path(&dir.path, 0).exists());
        assert!(chunk_path(&dir.path, 1).exists());
    }

    #[test]
    fn prune_chunks_keeps_start_block_until_its_long_segment_expires() {
        let dir = TestDirectory::new();
        let segment = Segment {
            kind: ActivityKind::Active,
            start_ms: 500,
            end_ms: ARCHIVE_BLOCK_MS * 2 + 500,
        };
        archive_segments(&dir.path, &[segment]).expect("archive");

        prune_chunks(&dir.path, ARCHIVE_BLOCK_MS * 2).expect("prune overlapping chunk");
        assert!(
            chunk_path(&dir.path, 0).exists(),
            "start chunk must remain while its segment overlaps the retained range"
        );

        prune_chunks(&dir.path, segment.end_ms).expect("prune expired chunk");
        assert!(!chunk_path(&dir.path, 0).exists());
    }

    #[test]
    fn prune_chunks_continues_after_one_failure() {
        let dir = TestDirectory::new();
        let seg0 = Segment {
            kind: ActivityKind::Active,
            start_ms: 500,
            end_ms: 900,
        };
        let seg1 = Segment {
            kind: ActivityKind::Afk,
            start_ms: ARCHIVE_BLOCK_MS + 500,
            end_ms: ARCHIVE_BLOCK_MS + 900,
        };
        let seg2 = Segment {
            kind: ActivityKind::Active,
            start_ms: ARCHIVE_BLOCK_MS * 2 + 500,
            end_ms: ARCHIVE_BLOCK_MS * 2 + 900,
        };
        archive_segments(&dir.path, &[seg0, seg1, seg2]).expect("archive");

        // Replace chunk 1's file with a directory so deleting it fails.
        fs::remove_file(chunk_path(&dir.path, 1)).expect("remove chunk1 file to replace with dir");
        fs::create_dir(chunk_path(&dir.path, 1)).expect("blocker directory");

        let cutoff = ARCHIVE_BLOCK_MS * 3; // expires all three blocks
        let result = prune_chunks(&dir.path, cutoff);

        assert!(result.is_err(), "failure on one chunk should surface");
        assert!(
            !chunk_path(&dir.path, 0).exists(),
            "chunk 0 still deleted despite chunk 1 failure"
        );
        assert!(
            !chunk_path(&dir.path, 2).exists(),
            "chunk 2 still deleted despite chunk 1 failure"
        );
        assert!(
            chunk_path(&dir.path, 1).exists(),
            "blocked chunk remains (as a directory)"
        );
    }
}
