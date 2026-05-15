use engagement_hub::telemetry::otlp_json::*;

fn main() {
    let span = fake_span_for_test("test-op", 100, StatusForTest::Ok);
    let data = spans_to_traces_data(&[span], &[], "test-scope");
    let json = serde_json::to_string_pretty(&data).unwrap();
    println!("{}", json);
}
