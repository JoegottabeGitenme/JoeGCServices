//! Results reporting and formatting.

use crate::metrics::TestResults;
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, Table};

/// Formats test results for output.
pub struct ResultsReport;

impl ResultsReport {
    /// Format results as a console table.
    pub fn format_table(results: &TestResults) -> String {
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![format!("Load Test Results: {}", results.config_name)]);

        table.add_row(vec!["Duration:", &format!("{:.1}s", results.duration_secs)]);
        table.add_row(vec![
            "Total Requests:",
            &format!("{}", results.total_requests),
        ]);
        table.add_row(vec![
            "Success Rate:",
            &format!(
                "{:.1}%",
                (results.successful_requests as f64 / results.total_requests as f64) * 100.0
            ),
        ]);
        table.add_row(vec![
            "Requests/sec:",
            &format!("{:.1}", results.requests_per_second),
        ]);

        table.add_row(vec!["", ""]);
        table.add_row(vec!["Latency (ms)", "p50 / p90 / p95 / p99 / max"]);
        table.add_row(vec![
            "",
            &format!(
                "{:.1} / {:.1} / {:.1} / {:.1} / {:.1}",
                results.latency_p50,
                results.latency_p90,
                results.latency_p95,
                results.latency_p99,
                results.latency_max
            ),
        ]);

        table.add_row(vec!["", ""]);
        table.add_row(vec![
            "Cache Hit Rate:",
            &format!("{:.1}%", results.cache_hit_rate),
        ]);
        table.add_row(vec![
            "Throughput:",
            &format!("{:.1} MB/s", results.bytes_per_second / 1_000_000.0),
        ]);

        table.to_string()
    }

    /// Format results as JSON.
    pub fn format_json(results: &TestResults) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(results)?)
    }

    /// Format results as CSV row.
    pub fn format_csv(results: &TestResults) -> String {
        format!(
            "{},{},{},{},{:.1},{:.1},{:.1},{:.1},{:.1}",
            chrono::Utc::now().to_rfc3339(),
            results.config_name,
            results.duration_secs,
            results.total_requests,
            results.requests_per_second,
            results.latency_p50,
            results.latency_p90,
            results.latency_p99,
            results.cache_hit_rate
        )
    }

    /// CSV header row.
    pub fn csv_header() -> &'static str {
        "timestamp,config,duration,requests,rps,p50,p90,p99,cache_hit_rate"
    }
}
