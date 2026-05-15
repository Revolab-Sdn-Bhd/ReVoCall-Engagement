use assert_cmd::Command;
use std::path::PathBuf;

fn trace_query() -> Command {
    let mut cmd = Command::new("bash");
    cmd.arg(bin_path());
    cmd
}

fn bin_path() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let p = PathBuf::from(&manifest).join("../../bin/trace-query");
    assert!(p.exists(), "trace-query not found at {p:?}; run: ln -s ../revolab-observability/tools/trace-query bin/trace-query");
    p
}

fn fixture_dir() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    PathBuf::from(&manifest).join("tests/fixtures")
}

#[test]
fn engagement_returns_spans_for_id() {
    let output = trace_query()
        .arg("engagement")
        .arg("eng-001")
        .env("TRACES_DIR", fixture_dir().to_str().unwrap())
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let spans = json.as_array().unwrap();
    assert_eq!(spans.len(), 2, "expected 2 spans for eng-001: {json}");
    for span in spans {
        assert_eq!(
            span["attrs"]["revolab.engagement_id"].as_str().unwrap(),
            "eng-001"
        );
    }
    let ops: Vec<&str> = spans.iter()
        .map(|s| s["operation"].as_str().unwrap())
        .collect();
    assert!(ops.contains(&"op-fast"),  "missing op-fast in {json}");
    assert!(ops.contains(&"op-error"), "missing op-error in {json}");
}

#[test]
fn trace_returns_spans_for_trace_id() {
    let output = trace_query()
        .arg("trace")
        .arg("03030303030303030303030303030303")
        .env("TRACES_DIR", fixture_dir().to_str().unwrap())
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let spans = json.as_array().unwrap();
    assert_eq!(spans.len(), 1, "{json}");
    assert_eq!(spans[0]["trace_id"].as_str().unwrap(), "03030303030303030303030303030303");
}

#[test]
fn slow_returns_spans_at_or_above_threshold() {
    let output = trace_query()
        .arg("slow")
        .arg("1000")
        .env("TRACES_DIR", fixture_dir().to_str().unwrap())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let spans = json.as_array().unwrap();
    // Only op-error at 1500ms should be >= 1000ms
    assert_eq!(spans.len(), 1, "{json}");
    assert_eq!(spans[0]["operation"].as_str().unwrap(), "op-error");
    assert!(spans[0]["duration_ms"].as_f64().unwrap() >= 1000.0);
}

#[test]
fn errors_returns_only_error_spans() {
    let output = trace_query()
        .arg("errors")
        .env("TRACES_DIR", fixture_dir().to_str().unwrap())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let spans = json.as_array().unwrap();
    assert_eq!(spans.len(), 1, "{json}");
    assert_eq!(spans[0]["status"].as_str().unwrap(), "STATUS_CODE_ERROR");
    assert_eq!(spans[0]["operation"].as_str().unwrap(), "op-error");
}
