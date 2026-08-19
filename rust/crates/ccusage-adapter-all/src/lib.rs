mod loader;
mod report;
mod types;

use ccusage_adapter_codex::CodexGroup;
#[cfg(test)]
use ccusage_adapter_codex::CodexModelUsage;
use ccusage_adapter_common::filter_loaded_entries_by_date;
use ccusage_core::*;

mod adapter {
    pub use ccusage_adapter_amp as amp;
    pub use ccusage_adapter_claude as claude;
    pub use ccusage_adapter_codebuff as codebuff;
    pub use ccusage_adapter_codex as codex;
    pub use ccusage_adapter_copilot as copilot;
    pub use ccusage_adapter_droid as droid;
    pub use ccusage_adapter_gemini as gemini;
    pub use ccusage_adapter_goose as goose;
    pub use ccusage_adapter_grok as grok;
    pub use ccusage_adapter_hermes as hermes;
    pub use ccusage_adapter_kilo as kilo;
    pub use ccusage_adapter_kimi as kimi;
    pub use ccusage_adapter_openclaw as openclaw;
    pub use ccusage_adapter_opencode as opencode;
    pub use ccusage_adapter_pi as pi;
    pub use ccusage_adapter_qwen as qwen;
}

use crate::{
    Result,
    cli::{AgentCommandArgs, AgentReportKind},
    print_json_or_jq, wants_json,
};

pub fn run(args: AgentCommandArgs) -> Result<()> {
    let kind = args.kind;
    let include_agents = args.by_agent || args.by_provider || !args.agent_selectors.is_empty();
    let filters = loader::AllFilters {
        agent_selectors: &args.agent_selectors,
        model_patterns: &args.model_patterns,
        by_provider: args.by_provider,
    };
    let shared = &args.shared;
    if let Some(sections) = args.sections.clone() {
        let sections = requested_sections(kind, sections);
        let mut result = loader::load_sections(&sections, shared, filters)?;
        if args.by_provider {
            for (_, rows) in &mut result.sections {
                *rows = types::group_rows_by_provider(std::mem::take(rows));
            }
        }
        if wants_json(shared) {
            return report::print_sections_report_json(
                &result.sections,
                kind,
                include_agents,
                args.by_provider,
                shared.jq.as_deref(),
                shared.no_cost,
            );
        }
        for (section_kind, rows) in &result.sections {
            report::print_table(
                rows,
                *section_kind,
                shared,
                result.detected_agents_for(*section_kind),
                args.by_provider,
                args.summary,
            )?;
        }
        return Ok(());
    }
    let mut result = loader::load_rows(kind, shared, filters)?;
    if args.summary {
        result.rows = types::summarize_rows(result.rows);
    }
    if args.by_provider {
        result.rows = types::group_rows_by_provider(result.rows);
    }
    if wants_json(shared) {
        let output = report::report_json_with_breakdowns(
            &result.rows,
            kind,
            include_agents,
            args.by_provider,
        );
        return print_json_or_jq(output, shared.jq.as_deref(), shared.no_cost);
    }
    report::print_table(
        &result.rows,
        kind,
        shared,
        &result.detected_agents,
        args.by_provider,
        args.summary,
    )
}

fn requested_sections(
    command_kind: AgentReportKind,
    sections: Vec<AgentReportKind>,
) -> Vec<AgentReportKind> {
    let mut requested = vec![command_kind];
    for section in [
        AgentReportKind::Daily,
        AgentReportKind::Weekly,
        AgentReportKind::Monthly,
        AgentReportKind::Session,
    ] {
        if section != command_kind && sections.contains(&section) {
            requested.push(section);
        }
    }
    requested
}

#[cfg(test)]
use loader::{
    AllFilters, aggregate_rows, codex_group_row, filter_row_models, load_agent_rows_parallel,
    load_rows, load_sections,
};
#[cfg(test)]
use report::{
    all_report_title, all_table_columns, all_table_row, provider_label, report_json,
    report_json_with_agents, report_json_with_breakdowns, sections_report_json,
};
#[cfg(test)]
use types::{AgentLoadSpec, AgentRows, AllRow, group_rows_by_provider, summarize_rows};

#[cfg(test)]
mod tests;
