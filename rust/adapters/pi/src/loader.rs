use std::{collections::HashSet, path::PathBuf};

use crate::{
    LoadedEntry, PricingMap, Result, cli::SharedArgs, collect_files_with_extension, debug_log,
    parse_tz, read_files_parallel,
};

use super::{parser, paths};

pub fn load_entries(
    shared: &SharedArgs,
    custom_path: Option<&str>,
    pricing: Option<&PricingMap>,
) -> Result<Vec<LoadedEntry>> {
    crate::progress::track_usage_load(
        crate::progress::UsageLoadAgent("pi-agent"),
        shared.json,
        || load_entries_inner(shared, custom_path, pricing),
    )
}

fn load_entries_inner(
    shared: &SharedArgs,
    custom_path: Option<&str>,
    pricing: Option<&PricingMap>,
) -> Result<Vec<LoadedEntry>> {
    load_entries_from_paths(
        shared,
        paths::paths(custom_path)?,
        pricing,
        PiLoadScope::Default,
    )
}

#[doc(hidden)]
pub fn load_entries_for_store_path(
    shared: &SharedArgs,
    store_path: &str,
    store_name: &str,
    pricing: Option<&PricingMap>,
) -> Result<Vec<LoadedEntry>> {
    load_entries_for_store_paths(
        shared,
        paths::named_store_paths(store_path)?,
        store_name,
        pricing,
    )
}

pub fn load_entries_for_store_paths(
    shared: &SharedArgs,
    store_paths: Vec<PathBuf>,
    store_name: &str,
    pricing: Option<&PricingMap>,
) -> Result<Vec<LoadedEntry>> {
    load_entries_from_paths(
        shared,
        store_paths,
        pricing,
        PiLoadScope::Named { store_name },
    )
}

#[derive(Clone, Copy)]
enum PiLoadScope<'a> {
    Default,
    Named { store_name: &'a str },
}

#[derive(Debug, Eq, Hash, PartialEq)]
enum PiDedupeKey {
    Message(String),
    Legacy {
        entry: String,
        provider: Option<String>,
    },
}

impl<'a> PiLoadScope<'a> {
    fn store_name(self) -> &'a str {
        match self {
            Self::Default => "pi",
            Self::Named { store_name } => store_name,
        }
    }

    fn debug_label(self) -> String {
        match self {
            Self::Default => "pi".to_string(),
            Self::Named { store_name } => format!("pi-format store '{store_name}'"),
        }
    }
}

fn load_entries_from_paths(
    shared: &SharedArgs,
    paths: Vec<PathBuf>,
    pricing: Option<&PricingMap>,
    scope: PiLoadScope<'_>,
) -> Result<Vec<LoadedEntry>> {
    let tz = parse_tz(shared.timezone.as_deref());
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let mut files = Vec::new();
        collect_files_with_extension(&path, "jsonl", &mut files);
        // Read session files in parallel; the first-wins dedup runs sequentially
        // over the original file order so the surviving record per id matches the
        // single-threaded read.
        let loaded = read_files_parallel(&files, shared.single_thread, |file| {
            let result = match scope {
                PiLoadScope::Default => {
                    parser::read_session_file(file, tz.as_ref(), shared.mode, pricing)
                }
                PiLoadScope::Named { store_name } => parser::read_session_file_for_store(
                    file,
                    &path,
                    tz.as_ref(),
                    shared.mode,
                    pricing,
                    store_name,
                ),
            };
            result.unwrap_or_else(|error| {
                let label = scope.debug_label();
                debug_log(
                    shared,
                    format!(
                        "Failed to read {label} session file {}: {error}",
                        file.display()
                    ),
                );
                Vec::new()
            })
        });
        for file_entries in loaded {
            for entry in file_entries {
                let id = if let Some(message_id) = entry.data.message.id.clone() {
                    PiDedupeKey::Message(message_id)
                } else {
                    let entry_id = match scope {
                        PiLoadScope::Default => parser::entry_id(&entry),
                        PiLoadScope::Named { .. } => {
                            parser::entry_id_for_store(scope.store_name(), &entry)
                        }
                    };
                    PiDedupeKey::Legacy {
                        entry: entry_id,
                        provider: entry.provider.clone(),
                    }
                };
                if seen.insert(id) {
                    entries.push(entry);
                }
            }
        }
    }
    entries.sort_by_key(|entry| entry.timestamp);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ccusage_test_support::fs_fixture;

    #[test]
    fn counts_a_resumed_message_id_once_across_session_files() {
        let fixture = fs_fixture!({
            "sessions/project-a/agent_original.jsonl": r#"{"type":"message","id":"msg-420","timestamp":"2026-01-02T00:00:00.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4","usage":{"input":400,"output":20}}}"#,
            "sessions/project-a/agent_resumed.jsonl": r#"{"type":"message","id":"msg-420","timestamp":"2026-01-03T00:00:00.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4","usage":{"input":400,"output":20}}}"#,
        });
        let shared = SharedArgs {
            mode: crate::cli::CostMode::Display,
            ..SharedArgs::default()
        };

        let entries = load_entries(
            &shared,
            Some(fixture.path("sessions").to_str().unwrap()),
            None,
        )
        .unwrap();
        let total = entries
            .iter()
            .map(|entry| {
                crate::total_usage_tokens(entry.data.message.usage) + entry.extra_total_tokens
            })
            .sum::<u64>();

        assert_eq!(entries.len(), 1);
        assert_eq!(total, 420);
    }

    #[test]
    fn counts_a_subagent_transcript_mirror_and_nested_source_once() {
        let fixture = fs_fixture!({
            "sessions/project-a/subagent-artifacts/worker_transcript.jsonl": r#"{"type":"message","id":"subagent-msg","timestamp":"2026-01-02T00:00:00.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4","usage":{"input":300,"output":20}}}"#,
            "sessions/project-a/subagent-artifacts/run-worker/session.jsonl": r#"{"type":"message","id":"subagent-msg","timestamp":"2026-01-02T00:00:00.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4","usage":{"input":300,"output":20}}}"#,
        });
        let shared = SharedArgs {
            mode: crate::cli::CostMode::Display,
            ..SharedArgs::default()
        };

        let entries = load_entries(
            &shared,
            Some(fixture.path("sessions").to_str().unwrap()),
            None,
        )
        .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].data.message.id.as_deref(), Some("subagent-msg"));
    }

    #[test]
    fn keeps_distinct_nested_subagent_message_ids() {
        let fixture = fs_fixture!({
            "sessions/project-a/subagent-artifacts/run-worker-a/session.jsonl": r#"{"type":"message","id":"subagent-msg-a","timestamp":"2026-01-02T00:00:00.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4","usage":{"input":200,"output":10}}}"#,
            "sessions/project-a/subagent-artifacts/run-worker-b/session.jsonl": r#"{"type":"message","id":"subagent-msg-b","timestamp":"2026-01-02T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4","usage":{"input":200,"output":10}}}"#,
        });
        let shared = SharedArgs {
            mode: crate::cli::CostMode::Display,
            ..SharedArgs::default()
        };

        let entries = load_entries(
            &shared,
            Some(fixture.path("sessions").to_str().unwrap()),
            None,
        )
        .unwrap();
        let ids = entries
            .iter()
            .filter_map(|entry| entry.data.message.id.as_deref())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["subagent-msg-a", "subagent-msg-b"]);
    }

    #[test]
    fn provider_metadata_does_not_split_copies_of_one_message_id() {
        let fixture = fs_fixture!({
            "sessions/project-a/agent_original.jsonl": r#"{"type":"message","id":"shared-msg","timestamp":"2026-01-02T00:00:00.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4","usage":{"input":100,"output":10}}}"#,
            "sessions/project-a/agent_resumed.jsonl": r#"{"type":"message","id":"shared-msg","timestamp":"2026-01-03T00:00:00.000Z","message":{"role":"assistant","provider":"openai-codex","model":"claude-sonnet-4","usage":{"input":100,"output":10}}}"#,
        });
        let shared = SharedArgs {
            mode: crate::cli::CostMode::Display,
            ..SharedArgs::default()
        };

        let entries = load_entries(
            &shared,
            Some(fixture.path("sessions").to_str().unwrap()),
            None,
        )
        .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].data.message.id.as_deref(), Some("shared-msg"));
    }
}
