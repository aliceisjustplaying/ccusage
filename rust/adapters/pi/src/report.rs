use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::{
    BucketKind, LoadedEntry, Result, SessionAccumulator, cli::AgentReportKind, cli::WeekDay,
    summarize_by_key, summarize_summaries_by_bucket, totals_json,
};

pub fn report_from_rows(rows: &[crate::UsageSummary], kind: AgentReportKind) -> Value {
    let rows_json = rows
        .iter()
        .map(|row| ccusage_core::agent_summary_json(row, kind, kind == AgentReportKind::Session))
        .collect::<Vec<_>>();
    json!({
        rows_key(kind): rows_json,
        "totals": totals_json(rows),
    })
}

pub fn summarize_entries(
    entries: &[LoadedEntry],
    kind: AgentReportKind,
) -> Result<Vec<crate::UsageSummary>> {
    match kind {
        AgentReportKind::Daily => summarize_by_key(
            entries,
            |entry| entry.date.clone(),
            |date| (date.to_string(), None),
        ),
        AgentReportKind::Monthly => {
            let daily = summarize_by_key(
                entries,
                |entry| entry.date.clone(),
                |date| (date.to_string(), None),
            )?;
            Ok(summarize_summaries_by_bucket(
                &daily,
                BucketKind::Monthly,
                WeekDay::Sunday,
            ))
        }
        AgentReportKind::Session => {
            let mut providers_by_session = BTreeMap::<&str, BTreeSet<Option<&str>>>::new();
            for entry in entries {
                providers_by_session
                    .entry(entry.session_id.as_ref())
                    .or_default()
                    .insert(entry.data.version.as_deref());
            }

            let mut groups = BTreeMap::<String, SessionAccumulator>::new();
            for entry in entries {
                let session_id = if providers_by_session
                    .get(entry.session_id.as_ref())
                    .is_some_and(|providers| providers.len() > 1)
                {
                    format!(
                        "{}@{}",
                        entry.session_id,
                        entry.data.version.as_deref().unwrap_or("unknown")
                    )
                } else {
                    entry.session_id.to_string()
                };
                groups.entry(session_id).or_default().add_entry(entry);
            }
            groups
                .into_iter()
                .map(|(session_id, group)| {
                    let mut summary = group.into_summary()?;
                    summary.session_id = Some(session_id);
                    Ok(summary)
                })
                .collect()
        }
        AgentReportKind::Weekly => {
            let daily = summarize_entries(entries, AgentReportKind::Daily)?;
            Ok(summarize_summaries_by_bucket(
                &daily,
                BucketKind::Weekly,
                WeekDay::Sunday,
            ))
        }
    }
}

fn rows_key(kind: AgentReportKind) -> &'static str {
    match kind {
        AgentReportKind::Daily => "daily",
        AgentReportKind::Weekly => "weekly",
        AgentReportKind::Monthly => "monthly",
        AgentReportKind::Session => "sessions",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_has_zero_totals_object() {
        let report = report_from_rows(&[], AgentReportKind::Daily);

        assert_eq!(report["daily"], json!([]));
        assert!(report["totals"].is_object());
        assert_eq!(report["totals"]["inputTokens"], json!(0));
        assert_eq!(report["totals"]["outputTokens"], json!(0));
        assert_eq!(report["totals"]["cacheCreationTokens"], json!(0));
        assert_eq!(report["totals"]["cacheReadTokens"], json!(0));
        assert_eq!(report["totals"]["totalTokens"], json!(0));
        assert_eq!(report["totals"]["totalCost"].as_f64(), Some(0.0));
    }
}
