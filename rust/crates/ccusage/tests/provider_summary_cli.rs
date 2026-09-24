use std::process::Command;

use ccusage_test_support::Fixture;
use serde_json::{Value, json};

fn fixture() -> Fixture {
    let fixture = Fixture::new();
    let records = [
        ("2026-07-01", Some("excluded"), 100, 100.0),
        ("2026-08-01", Some("alpha"), 10, 1.0),
        ("2026-08-02", Some("beta"), 20, 2.0),
        ("2026-08-03", Some("alpha"), 30, 3.0),
        ("2026-08-04", None, 40, 4.0),
    ];
    let records = records
        .into_iter()
        .map(|(date, provider, input, cost)| {
            json!({
                "type": "message", "timestamp": format!("{date}T12:00:00Z"),
                "message": {"role": "assistant", "provider": provider, "model": "same-model",
                    "usage": {"input": input, "output": 1, "cost": {"total": cost}}}
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let _ = fixture.write_file("pi/sessions/project/session.jsonl", records);
    let _ = fixture.write_file(
        "named/project/session.jsonl",
        json!({
            "type": "message", "timestamp": "2026-08-02T12:00:00Z",
            "message": {"role": "assistant", "provider": "alpha", "model": "same-model",
                "usage": {"input": 50, "output": 1, "cost": {"total": 5.0}}}
        })
        .to_string(),
    );
    let _ = fixture.write_file(
        "ccusage.json",
        json!({
            "pi": {"stores": [{"name": "omp", "path": fixture.path("named")}]}
        })
        .to_string(),
    );
    fixture
}

fn run(fixture: &Fixture, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_ccusage"))
        .env_clear()
        .env("HOME", fixture.path("home"))
        .env("USERPROFILE", fixture.path("home"))
        .env("XDG_CONFIG_HOME", fixture.path("config"))
        .env("PI_AGENT_DIR", fixture.path("pi"))
        .env("LOG_LEVEL", "0")
        .env("NO_COLOR", "1")
        .env("COLUMNS", "120")
        .args(args)
        .args([
            "--offline",
            "--no-color",
            "--timezone",
            "UTC",
            "--mode",
            "display",
        ])
        .args(["-s", "2026-08-01", "-u", "2026-08-31", "--config"])
        .arg(fixture.path("ccusage.json"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn provider_summary_cli_preserves_totals_and_named_stores() {
    let fixture = fixture();
    for kind in ["daily", "weekly", "monthly", "session"] {
        let baseline: Value = serde_json::from_str(&run(&fixture, &[kind, "--json"])).unwrap();
        let result: Value = serde_json::from_str(&run(
            &fixture,
            &[
                kind,
                "--by-provider",
                "--summary",
                "--breakdown",
                "--by-agent",
                "--json",
            ],
        ))
        .unwrap();
        assert_eq!(result["totals"], baseline["totals"], "{kind}");
        assert_eq!(result["totals"]["totalTokens"], 155);
        assert_eq!(result["totals"]["totalCost"], 15.0);
        let rows = result[kind].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let pi = rows.iter().find(|row| row["agent"] == "pi").unwrap();
        assert_eq!(pi["period"], "Summary");
        assert_eq!(pi["inputTokens"], 100);
        let providers = pi["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 3);
        let alpha = providers
            .iter()
            .find(|row| row["provider"] == "alpha")
            .unwrap();
        assert_eq!(alpha["inputTokens"], 40);
        assert_eq!(alpha["totalCost"], 4.0);
        assert!(
            providers
                .iter()
                .any(|row| row["provider"] == "Provider not recorded")
        );
        let omp = rows.iter().find(|row| row["agent"] == "omp").unwrap();
        assert_eq!(omp["providers"][0]["provider"], "alpha");
        assert_eq!(omp["totalCost"], 5.0);
        let table = run(
            &fixture,
            &[kind, "--by-provider", "--summary", "--breakdown"],
        );
        assert!(table.contains("Provider"), "{table}");
        assert!(table.contains("alpha") && table.contains("beta"));
        assert!(!table.contains("unknown"));
        assert!(table.contains("Total counts usage once."));
        assert!(table.contains("same-model") && table.contains("Summary"));
        insta::assert_snapshot!(format!("provider_summary_{kind}"), table);
    }
}

#[test]
fn summary_alone_and_sections_preserve_totals() {
    let fixture = fixture();
    let result: Value = serde_json::from_str(&run(
        &fixture,
        &[
            "daily",
            "--summary",
            "--sections",
            "weekly,monthly,session",
            "--json",
        ],
    ))
    .unwrap();
    for kind in ["daily", "weekly", "monthly", "session"] {
        assert_eq!(result[kind].as_array().unwrap().len(), 1);
        assert_eq!(result[kind][0]["totalTokens"], 155);
        assert_eq!(result[kind][0]["totalCost"], 15.0);
        assert!(result[kind][0].get("provider").is_none());
    }
}

#[test]
fn provider_grouping_keeps_periods_without_summary() {
    let fixture = fixture();
    let result: Value =
        serde_json::from_str(&run(&fixture, &["daily", "--by-provider", "--json"])).unwrap();
    assert_eq!(result["daily"].as_array().unwrap().len(), 5);
    assert!(
        result["daily"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["period"] != "Summary")
    );
    assert_eq!(result["totals"]["totalTokens"], 155);
}

#[test]
fn provider_summary_supports_compact_no_cost_output() {
    let fixture = fixture();
    let table = run(
        &fixture,
        &[
            "daily",
            "--by-provider",
            "--summary",
            "--breakdown",
            "--compact",
            "--no-cost",
        ],
    );
    assert!(table.contains("alpha") && table.contains("beta"));
    assert!(!table.contains("Cost (USD)"));
    let result: Value = serde_json::from_str(&run(
        &fixture,
        &["daily", "--by-provider", "--summary", "--no-cost", "--json"],
    ))
    .unwrap();
    assert!(result["totals"].get("totalCost").is_none());
    assert_eq!(result["totals"]["totalTokens"], 155);
}
