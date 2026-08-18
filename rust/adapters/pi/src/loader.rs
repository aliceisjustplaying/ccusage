use std::{collections::HashSet, path::PathBuf};

use jiff::tz::TimeZone as JiffTimeZone;

use crate::{
    LoadedEntry, PiEntry, PricingMap, Result, cli::SharedArgs, collect_files_with_extension,
    debug_log, parse_tz, read_files_parallel,
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
        || load_entries_inner::<LoadedEntry>(shared, custom_path, pricing),
    )
}

#[doc(hidden)]
pub fn load_entries_with_provider(
    shared: &SharedArgs,
    custom_path: Option<&str>,
    pricing: Option<&PricingMap>,
) -> Result<Vec<PiEntry>> {
    crate::progress::track_usage_load(
        crate::progress::UsageLoadAgent("pi-agent"),
        shared.json,
        || load_entries_inner::<PiEntry>(shared, custom_path, pricing),
    )
}

fn load_entries_inner<T: PiLoadedEntry>(
    shared: &SharedArgs,
    custom_path: Option<&str>,
    pricing: Option<&PricingMap>,
) -> Result<Vec<T>> {
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
    load_entries_from_paths::<LoadedEntry>(
        shared,
        store_paths,
        pricing,
        PiLoadScope::Named { store_name },
    )
}

#[doc(hidden)]
pub fn load_entries_for_store_paths_with_provider(
    shared: &SharedArgs,
    store_paths: Vec<PathBuf>,
    store_name: &str,
    pricing: Option<&PricingMap>,
) -> Result<Vec<PiEntry>> {
    load_entries_from_paths::<PiEntry>(
        shared,
        store_paths,
        pricing,
        PiLoadScope::Named { store_name },
    )
}

trait PiLoadedEntry: Send + Sized {
    fn read_default(
        path: &std::path::Path,
        tz: Option<&JiffTimeZone>,
        shared: &SharedArgs,
        pricing: Option<&PricingMap>,
    ) -> Result<Vec<Self>>;

    fn read_named(
        path: &std::path::Path,
        store_root: &std::path::Path,
        tz: Option<&JiffTimeZone>,
        shared: &SharedArgs,
        pricing: Option<&PricingMap>,
        store_name: &str,
    ) -> Result<Vec<Self>>;

    fn loaded(&self) -> &LoadedEntry;

    fn provider(&self) -> Option<&str> {
        None
    }
}

impl PiLoadedEntry for LoadedEntry {
    fn read_default(
        path: &std::path::Path,
        tz: Option<&JiffTimeZone>,
        shared: &SharedArgs,
        pricing: Option<&PricingMap>,
    ) -> Result<Vec<Self>> {
        parser::read_session_file(path, tz, shared.mode, pricing)
    }

    fn read_named(
        path: &std::path::Path,
        store_root: &std::path::Path,
        tz: Option<&JiffTimeZone>,
        shared: &SharedArgs,
        pricing: Option<&PricingMap>,
        store_name: &str,
    ) -> Result<Vec<Self>> {
        parser::read_session_file_for_store(path, store_root, tz, shared.mode, pricing, store_name)
    }

    fn loaded(&self) -> &LoadedEntry {
        self
    }
}

impl PiLoadedEntry for PiEntry {
    fn read_default(
        path: &std::path::Path,
        tz: Option<&JiffTimeZone>,
        shared: &SharedArgs,
        pricing: Option<&PricingMap>,
    ) -> Result<Vec<Self>> {
        parser::read_session_file_with_provider(path, tz, shared.mode, pricing)
    }

    fn read_named(
        path: &std::path::Path,
        store_root: &std::path::Path,
        tz: Option<&JiffTimeZone>,
        shared: &SharedArgs,
        pricing: Option<&PricingMap>,
        store_name: &str,
    ) -> Result<Vec<Self>> {
        parser::read_session_file_for_store_with_provider(
            path,
            store_root,
            tz,
            shared.mode,
            pricing,
            store_name,
        )
    }

    fn loaded(&self) -> &LoadedEntry {
        &self.entry
    }

    fn provider(&self) -> Option<&str> {
        self.provider.as_deref()
    }
}

#[derive(Clone, Copy)]
enum PiLoadScope<'a> {
    Default,
    Named { store_name: &'a str },
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

fn load_entries_from_paths<T: PiLoadedEntry>(
    shared: &SharedArgs,
    paths: Vec<PathBuf>,
    pricing: Option<&PricingMap>,
    scope: PiLoadScope<'_>,
) -> Result<Vec<T>> {
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
                PiLoadScope::Default => T::read_default(file, tz.as_ref(), shared, pricing),
                PiLoadScope::Named { store_name } => {
                    T::read_named(file, &path, tz.as_ref(), shared, pricing, store_name)
                }
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
                let id = match scope {
                    PiLoadScope::Default => parser::entry_id(entry.loaded()),
                    PiLoadScope::Named { .. } => {
                        parser::entry_id_for_store(scope.store_name(), entry.loaded())
                    }
                };
                let id = match entry.provider() {
                    Some(provider) => format!("{id}:provider:{}:{provider}", provider.len()),
                    None => id,
                };
                if seen.insert(id) {
                    entries.push(entry);
                }
            }
        }
    }
    entries.sort_by_key(|entry| entry.loaded().timestamp);
    Ok(entries)
}
