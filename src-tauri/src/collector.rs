use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::parser::{
    event_fingerprint, parse_legacy_jsonl_line, parse_newline_terminated_bytes_with_state,
    ParseStats, ParserState, UsageEvent,
};

#[derive(Debug, Clone, Default)]
pub struct SourceCheckpoint {
    pub source_key: String,
    pub byte_offset: u64,
    pub source_active: bool,
    pub parser_state_json: String,
}

#[derive(Debug, Clone, Default)]
pub struct PersistedCheckpoint {
    pub byte_offset: u64,
    pub parser_state_json: String,
}

#[derive(Debug, Clone, Default)]
pub struct CollectionSummary {
    pub events: Vec<UsageEvent>,
    pub(crate) superseded_fingerprints: Vec<String>,
    pub checkpoints: Vec<SourceCheckpoint>,
    pub stats: ParseStats,
    pub interrupted_sources: Vec<String>,
    pub skipped_symlinks: u64,
}

fn is_active_source(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    !lower.contains("archive") && !lower.contains("archived")
}

#[derive(Default)]
struct TraversalState {
    visited_directories: HashSet<PathBuf>,
    skipped_symlinks: u64,
}

fn collect_jsonl_paths(
    directory: &Path,
    output: &mut Vec<PathBuf>,
    state: &mut TraversalState,
) -> Result<(), String> {
    let canonical = fs::canonicalize(directory)
        .map_err(|_| "unable to access a Codex data directory during scan".to_string())?;
    if !state.visited_directories.insert(canonical) {
        return Err("recursive Codex directory link detected during scan".into());
    }
    let entries = fs::read_dir(directory)
        .map_err(|_| "unable to read a Codex data directory during scan".to_string())?;
    for entry in entries {
        let entry = entry.map_err(|_| "unable to read a Codex directory entry".to_string())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "unable to inspect a Codex directory entry".to_string())?;
        if file_type.is_symlink() {
            state.skipped_symlinks += 1;
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_jsonl_paths(&path, output, state)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        {
            output.push(path);
        }
    }
    Ok(())
}

fn source_key(path: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(path.to_string_lossy().as_bytes());
    format!("source_{:x}", digest.finalize())
}

fn has_complete_unterminated_record(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.last().is_some_and(|byte| *byte == b'\n') {
        return false;
    }
    let tail_start = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    let tail = &bytes[tail_start..];
    !tail.iter().all(|byte| byte.is_ascii_whitespace())
        && serde_json::from_slice::<serde_json::Value>(tail).is_ok()
}

fn checkpoint_signature(file: &mut File, offset: u64) -> std::io::Result<String> {
    let length = offset.min(4096) as usize;
    let mut bytes = vec![0; length];
    let mut digest = Sha256::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        digest.update(metadata.dev().to_le_bytes());
        digest.update(metadata.ino().to_le_bytes());
    }
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut bytes)?;
    digest.update(&bytes);
    if offset > length as u64 {
        file.seek(SeekFrom::Start(offset - length as u64))?;
        file.read_exact(&mut bytes)?;
        digest.update(&bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn scan_codex_home(
    home: &Path,
    previous: &HashMap<String, u64>,
) -> Result<CollectionSummary, String> {
    let previous_with_state = previous
        .iter()
        .map(|(key, offset)| {
            (
                key.clone(),
                PersistedCheckpoint {
                    byte_offset: *offset,
                    parser_state_json: String::new(),
                },
            )
        })
        .collect();
    scan_codex_home_with_state(home, &previous_with_state)
}

pub fn scan_codex_home_with_state(
    home: &Path,
    previous: &HashMap<String, PersistedCheckpoint>,
) -> Result<CollectionSummary, String> {
    scan_sources(home, previous, false)
}

pub(crate) fn scan_historical_codex_home(home: &Path) -> Result<CollectionSummary, String> {
    scan_sources(home, &HashMap::new(), true)
}

fn scan_sources(
    home: &Path,
    previous: &HashMap<String, PersistedCheckpoint>,
    reconcile_legacy_rows: bool,
) -> Result<CollectionSummary, String> {
    let mut paths = Vec::new();
    let mut traversal = TraversalState::default();
    collect_jsonl_paths(home, &mut paths, &mut traversal)?;
    paths.sort_by_key(|path| (!is_active_source(path), source_key(path)));
    let mut summary = CollectionSummary {
        skipped_symlinks: traversal.skipped_symlinks,
        ..CollectionSummary::default()
    };
    let mut seen = HashSet::new();
    let mut legacy_fingerprints = HashSet::new();
    for path in paths {
        let key = source_key(&path);
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                summary.interrupted_sources.push(key);
                continue;
            }
        };
        let size = metadata.len();
        let previous_checkpoint = previous.get(&key);
        let requested_offset = previous_checkpoint
            .map(|checkpoint| checkpoint.byte_offset)
            .unwrap_or(0);
        let mut parser_state = previous_checkpoint
            .and_then(|checkpoint| {
                serde_json::from_str::<ParserState>(&checkpoint.parser_state_json).ok()
            })
            .unwrap_or_default();
        let mut offset = if requested_offset > size {
            // A rotated or truncated file must not reuse cumulative parser
            // state from bytes that no longer exist.
            parser_state = ParserState::default();
            0
        } else {
            requested_offset
        };
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(_) => {
                summary.interrupted_sources.push(key);
                continue;
            }
        };
        if let Some(signature) = parser_state.source_signature.as_ref() {
            match checkpoint_signature(&mut file, offset) {
                Ok(actual) if &actual == signature => {}
                Ok(_) => {
                    offset = 0;
                    parser_state = ParserState::default();
                }
                Err(_) => {
                    summary.interrupted_sources.push(key);
                    continue;
                }
            }
        }
        if file.seek(SeekFrom::Start(offset)).is_err() {
            summary.interrupted_sources.push(key);
            continue;
        }
        let mut reader = BufReader::new((&mut file).take(size - offset));
        let mut line = Vec::new();
        let mut legacy_state = ParserState::default();
        let mut next_offset = offset;
        let mut read_failed = false;
        loop {
            line.clear();
            let bytes_read = match reader.read_until(b'\n', &mut line) {
                Ok(bytes_read) => bytes_read,
                Err(_) => {
                    read_failed = true;
                    break;
                }
            };
            if bytes_read == 0 {
                break;
            }
            let terminal_newline = line.last().is_some_and(|byte| *byte == b'\n');
            let complete_unterminated_record =
                !terminal_newline && has_complete_unterminated_record(&line);
            if !terminal_newline && !complete_unterminated_record {
                if !line.iter().all(|byte| byte.is_ascii_whitespace()) {
                    summary.stats.partial_line_retries += 1;
                }
                break;
            }
            if complete_unterminated_record {
                // A writer can exit after flushing a complete JSON object but before its newline.
                // Once the tail is valid JSON, consuming it is safe and prevents the final usage
                // record from being stranded at the same checkpoint forever.
                line.push(b'\n');
            }
            if reconcile_legacy_rows {
                let text = String::from_utf8_lossy(&line);
                if let Some(event) = parse_legacy_jsonl_line(&text, &mut legacy_state) {
                    legacy_fingerprints.insert(event_fingerprint(&event));
                }
            }
            let (events, stats) =
                parse_newline_terminated_bytes_with_state(&line, &mut parser_state);
            summary.stats.imported_records += stats.imported_records;
            summary.stats.partial_line_retries += stats.partial_line_retries;
            summary.stats.rejected_records += stats.rejected_records;
            for event in events {
                if seen.insert(event_fingerprint(&event)) {
                    summary.events.push(event);
                }
            }
            next_offset += bytes_read as u64;
        }
        if read_failed {
            summary.interrupted_sources.push(key);
            continue;
        }
        match checkpoint_signature(&mut file, next_offset) {
            Ok(signature) => parser_state.source_signature = Some(signature),
            Err(_) => {
                summary.interrupted_sources.push(key);
                continue;
            }
        }
        summary.checkpoints.push(SourceCheckpoint {
            source_key: key,
            byte_offset: next_offset.min(size),
            source_active: is_active_source(&path),
            parser_state_json: serde_json::to_string(&parser_state).unwrap_or_else(|_| "{}".into()),
        });
    }
    if reconcile_legacy_rows {
        summary.superseded_fingerprints = legacy_fingerprints.difference(&seen).cloned().collect();
    }
    Ok(summary)
}

pub fn stats_add(left: &mut ParseStats, right: &ParseStats) {
    left.imported_records += right.imported_records;
    left.partial_line_retries += right.partial_line_retries;
    left.rejected_records += right.rejected_records;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn resumes_a_same_size_replacement_from_the_beginning() {
        let root = std::env::temp_dir().join(format!("nerftrack-rotation-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let source = root.join("session.jsonl");
        let replacement = root.join("replacement");
        let line = r#"{"request_id":"r1","timestamp":1735689600,"model":"gpt-5","usage":{"input_tokens":10}}"#;
        fs::write(&source, format!("{line}\n")).unwrap();
        let before = scan_codex_home_with_state(&root, &HashMap::new()).unwrap();
        let previous = before
            .checkpoints
            .into_iter()
            .map(|checkpoint| {
                (
                    checkpoint.source_key,
                    PersistedCheckpoint {
                        byte_offset: checkpoint.byte_offset,
                        parser_state_json: checkpoint.parser_state_json,
                    },
                )
            })
            .collect();
        fs::write(&replacement, format!("{}\n", line.replace("r1", "r2"))).unwrap();
        fs::remove_file(&source).unwrap();
        fs::rename(replacement, source).unwrap();
        let after = scan_codex_home_with_state(&root, &previous).unwrap();
        assert_eq!(after.events.len(), 1);
        assert_eq!(after.events[0].request_id.as_deref(), Some("r2"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scans_with_byte_offsets_and_active_files_first() {
        let root = std::env::temp_dir().join(format!("nerftrack-collector-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("archive")).expect("directory");
        let active = root.join("session.jsonl");
        let archived = root.join("archive/session.jsonl");
        let line = r#"{"request_id":"r1","turn_id":"t1","timestamp":1735689600,"model":"gpt-5-codex","usage":{"input_tokens":10,"output_tokens":4}}"#;
        File::create(&active)
            .expect("active")
            .write_all(format!("{line}\n").as_bytes())
            .expect("write");
        File::create(&archived)
            .expect("archived")
            .write_all(format!("{line}\n").as_bytes())
            .expect("write");
        let first = scan_codex_home(&root, &HashMap::new()).expect("scan");
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.stats.imported_records, 2);
        let previous: HashMap<_, _> = first
            .checkpoints
            .iter()
            .map(|checkpoint| (checkpoint.source_key.clone(), checkpoint.byte_offset))
            .collect();
        let second = scan_codex_home(&root, &previous).expect("second scan");
        assert!(second.events.is_empty());
        assert_eq!(second.stats.imported_records, 0);
        assert!(!first.checkpoints[0]
            .source_key
            .contains(root.to_string_lossy().as_ref()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn imports_completed_final_record_without_trailing_newline_after_retry() {
        let root = std::env::temp_dir().join(format!(
            "nerftrack-collector-final-record-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("directory");
        let active = root.join("session.jsonl");
        let first_line = r#"{"request_id":"r1","turn_id":"t1","timestamp":1735689600,"model":"gpt-5-codex","usage":{"input_tokens":10,"output_tokens":4}}"#;
        let final_line = r#"{"request_id":"r2","turn_id":"t2","timestamp":1735689660,"model":"gpt-5-codex","usage":{"input_tokens":12,"output_tokens":5}}"#;
        let split = final_line.len() / 2;

        fs::write(&active, format!("{first_line}\n{}", &final_line[..split]))
            .expect("partial write");
        let first = scan_codex_home_with_state(&root, &HashMap::new()).expect("first scan");
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.stats.partial_line_retries, 1);

        let previous: HashMap<_, _> = first
            .checkpoints
            .iter()
            .map(|checkpoint| {
                (
                    checkpoint.source_key.clone(),
                    PersistedCheckpoint {
                        byte_offset: checkpoint.byte_offset,
                        parser_state_json: checkpoint.parser_state_json.clone(),
                    },
                )
            })
            .collect();

        fs::write(&active, format!("{first_line}\n{final_line}")).expect("final write");
        let second = scan_codex_home_with_state(&root, &previous).expect("second scan");
        assert_eq!(second.events.len(), 1);
        assert_eq!(second.events[0].request_id.as_deref(), Some("r2"));
        assert_eq!(second.stats.partial_line_retries, 0);
        assert_eq!(
            second.checkpoints[0].byte_offset,
            fs::metadata(&active).expect("metadata").len()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn empty_directory_is_distinct_from_an_unreadable_root() {
        let root = std::env::temp_dir().join(format!(
            "nerftrack-collector-empty (unicode-✓)-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("empty directory");
        let empty = scan_codex_home(&root, &HashMap::new()).expect("empty scan");
        assert!(empty.events.is_empty());
        assert!(empty.checkpoints.is_empty());

        let file_root = root.join("not-a-directory.jsonl");
        fs::write(&file_root, b"{}").expect("file root");
        assert!(scan_codex_home(&file_root, &HashMap::new()).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn skips_recursive_directory_links_and_surfaces_the_skip() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "nerftrack-collector-links (private)-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("directory");
        let source = root.join("session (✓).jsonl");
        let line = r#"{"request_id":"r1","turn_id":"t1","timestamp":1735689600,"model":"gpt-5-codex","usage":{"input_tokens":10,"output_tokens":4}}"#;
        fs::write(&source, format!("{line}\n")).expect("source");
        symlink(&root, root.join("cycle")).expect("cycle link");
        let result = scan_codex_home(&root, &HashMap::new()).expect("safe scan");
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.skipped_symlinks, 1);
        let _ = fs::remove_dir_all(root);
    }
}
