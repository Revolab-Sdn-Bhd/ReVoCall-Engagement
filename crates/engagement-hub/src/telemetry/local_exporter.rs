//! Local JSONL file exporter for OTLP traces.
//!
//! Writes one OTLP JSON envelope per line to `.traces/<service>-<YYYY-MM-DD>.jsonl`.
//! Supports daily rotation, a 100 MB size cap, and 7-day file retention.

use std::fs::{File, OpenOptions};
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};

use crate::telemetry::otlp_json::{KvAttribute, spans_to_traces_data};

const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug)]
pub struct JsonlFileExporter {
    service_name: &'static str,
    traces_dir: PathBuf,
    current_date: NaiveDate,
    current_file: Option<File>,
    size_cap: u64,
    drop_counter: Option<prometheus::IntCounter>,
    resource_attrs: Vec<KvAttribute>,
}

impl JsonlFileExporter {
    pub fn new(service_name: &'static str, resource_attrs: Vec<KvAttribute>) -> Self {
        Self::new_inner(
            service_name,
            PathBuf::from(".traces"),
            chrono::Local::now().date_naive(),
            MAX_FILE_BYTES,
            None,
            resource_attrs,
        )
    }

    /// Production constructor with drop counter.
    pub fn new_with_counter(
        service_name: &'static str,
        resource_attrs: Vec<KvAttribute>,
        drop_counter: prometheus::IntCounter,
    ) -> Self {
        Self::new_inner(
            service_name,
            PathBuf::from(".traces"),
            chrono::Local::now().date_naive(),
            MAX_FILE_BYTES,
            Some(drop_counter),
            resource_attrs,
        )
    }

    fn new_inner(
        service_name: &'static str,
        traces_dir: PathBuf,
        current_date: NaiveDate,
        size_cap: u64,
        drop_counter: Option<prometheus::IntCounter>,
        resource_attrs: Vec<KvAttribute>,
    ) -> Self {
        Self {
            service_name,
            traces_dir,
            current_date,
            current_file: None,
            size_cap,
            drop_counter,
            resource_attrs,
        }
    }

    #[cfg(test)]
    pub fn new_for_test(service_name: &'static str, dir: PathBuf) -> Self {
        Self::new_inner(
            service_name,
            dir,
            chrono::Local::now().date_naive(),
            MAX_FILE_BYTES,
            None,
            vec![],
        )
    }

    #[cfg(test)]
    pub fn new_for_test_with_date(
        service_name: &'static str,
        dir: PathBuf,
        date: NaiveDate,
    ) -> Self {
        Self::new_inner(service_name, dir, date, MAX_FILE_BYTES, None, vec![])
    }

    #[cfg(test)]
    pub fn new_for_test_with_size_cap(
        service_name: &'static str,
        dir: PathBuf,
        cap: u64,
        counter: prometheus::IntCounter,
    ) -> Self {
        Self::new_inner(
            service_name,
            dir,
            chrono::Local::now().date_naive(),
            cap,
            Some(counter),
            vec![],
        )
    }

    #[cfg(test)]
    pub fn advance_date_for_test(&mut self, date: NaiveDate) {
        self.current_date = date;
        self.current_file = None;
    }

    fn file_path(&self, date: NaiveDate) -> PathBuf {
        self.traces_dir
            .join(format!("{}-{}.jsonl", self.service_name, date))
    }

    fn ensure_file(&mut self) -> std::io::Result<&mut File> {
        // In tests, current_date is controlled by advance_date_for_test.
        // In production, update current_date from the live clock so daily rotation works.
        #[cfg(not(test))]
        {
            let today = chrono::Local::now().date_naive();
            if today != self.current_date {
                self.current_date = today;
                self.current_file = None;
            }
        }

        if self.current_file.is_none() {
            std::fs::create_dir_all(&self.traces_dir)?;
            let path = self.file_path(self.current_date);
            let file = OpenOptions::new().create(true).append(true).open(path)?;
            self.current_file = Some(file);
        }
        Ok(self.current_file.as_mut().unwrap())
    }

    fn write_batch(&mut self, batch: &[SpanData]) -> ExportResult {
        // Ensure the file is open (may update self.current_date / self.current_file).
        self.ensure_file()
            .map_err(|e| opentelemetry::trace::TraceError::Other(Box::new(e)))?;

        // Check size cap — read from the open file without keeping a borrow alive.
        let size = self
            .current_file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map(|m| m.len())
            .unwrap_or(0);

        if size >= self.size_cap {
            let batch_len = batch.len();
            if let Some(c) = &self.drop_counter {
                c.inc_by(batch_len as u64);
            }
            return Ok(());
        }

        // Serialize spans into a single JSONL line.
        let data = spans_to_traces_data(batch, &self.resource_attrs, "engagement-hub");
        let mut line = serde_json::to_string(&data)
            .map_err(|e| opentelemetry::trace::TraceError::Other(Box::new(e)))?;
        line.push('\n');

        self.current_file
            .as_mut()
            .unwrap()
            .write_all(line.as_bytes())
            .map_err(|e| opentelemetry::trace::TraceError::Other(Box::new(e)))?;
        Ok(())
    }

    /// Deletes `.jsonl` files in `dir` whose embedded date is older than
    /// `retention_days` before `today`.
    ///
    /// File naming convention: `<service-name>-YYYY-MM-DD.jsonl`
    pub fn purge_old_files(dir: &Path, today: NaiveDate, retention_days: i64) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.ends_with(".jsonl") {
                continue;
            }
            // The date is always the last 10 chars before ".jsonl": YYYY-MM-DD
            let without_ext = name_str.trim_end_matches(".jsonl");
            if without_ext.len() < 10 {
                continue;
            }
            let date_str = &without_ext[without_ext.len() - 10..];
            if let Ok(file_date) = date_str.parse::<NaiveDate>()
                && (today - file_date).num_days() > retention_days
            {
                if let Err(e) = std::fs::remove_file(entry.path()) {
                    eprintln!(
                        "[otel:local] failed to purge old trace file {:?}: {e}",
                        entry.path()
                    );
                }
            }
        }
    }
}

impl SpanExporter for JsonlFileExporter {
    fn export(
        &mut self,
        batch: Vec<SpanData>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ExportResult> + Send + 'static>> {
        let result = self.write_batch(&batch);
        Box::pin(std::future::ready(result))
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::otlp_json::{StatusForTest, fake_span_for_test};
    use std::path::PathBuf;

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eh-test-traces-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_export(
        exporter: &mut JsonlFileExporter,
        spans: Vec<opentelemetry_sdk::export::trace::SpanData>,
    ) {
        futures::executor::block_on(exporter.export(spans)).unwrap();
    }

    #[test]
    fn creates_file_and_writes_valid_jsonl() {
        let dir = test_dir();
        let mut exp = JsonlFileExporter::new_for_test("test-svc", dir.clone());

        run_export(
            &mut exp,
            vec![fake_span_for_test("op", 10, StatusForTest::Ok)],
        );

        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(files.len(), 1, "expected one file");
        let path = files[0].as_ref().unwrap().path();
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1, "expected one JSONL line");
        let _: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON");
        assert!(content.contains("resourceSpans"));
    }

    #[test]
    fn daily_rotation_opens_new_file() {
        let dir = test_dir();
        let date_a = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let date_b = chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();

        let mut exp = JsonlFileExporter::new_for_test_with_date("test-svc", dir.clone(), date_a);
        run_export(
            &mut exp,
            vec![fake_span_for_test("op", 5, StatusForTest::Ok)],
        );

        exp.advance_date_for_test(date_b);
        run_export(
            &mut exp,
            vec![fake_span_for_test("op2", 5, StatusForTest::Ok)],
        );

        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        files.sort();
        assert_eq!(files.len(), 2, "expected two daily files: {files:?}");
        assert!(files[0].contains("2024-01-01"), "{files:?}");
        assert!(files[1].contains("2024-01-02"), "{files:?}");
    }

    #[test]
    fn size_cap_stops_writes_and_increments_counter() {
        let dir = test_dir();
        let counter = prometheus::IntCounter::new("test_dropped_local", "test").unwrap();
        let mut exp = JsonlFileExporter::new_for_test_with_size_cap(
            "test-svc",
            dir.clone(),
            10, // 10 bytes cap — tiny
            counter.clone(),
        );

        // First export — file starts empty, should succeed
        run_export(
            &mut exp,
            vec![fake_span_for_test("op", 5, StatusForTest::Ok)],
        );
        // Second export — file exceeds cap, should be dropped
        run_export(
            &mut exp,
            vec![fake_span_for_test("op2", 5, StatusForTest::Ok)],
        );

        assert!(counter.get() > 0, "expected dropped counter > 0");
    }

    #[test]
    fn retention_deletes_files_older_than_7_days() {
        let dir = test_dir();
        let today = chrono::Local::now().date_naive();

        for days_ago in 1u64..=9 {
            let d = today - chrono::Duration::days(days_ago as i64);
            let path = dir.join(format!("test-svc-{}.jsonl", d));
            std::fs::write(&path, "{}").unwrap();
        }

        JsonlFileExporter::purge_old_files(&dir, today, 7);

        let remaining: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();

        assert_eq!(remaining.len(), 7, "expected 7 files, got: {remaining:?}");
        for name in &remaining {
            let date_part = name
                .trim_start_matches("test-svc-")
                .trim_end_matches(".jsonl");
            let d: chrono::NaiveDate = date_part.parse().unwrap();
            assert!(
                (today - d).num_days() <= 7,
                "file older than 7 days not purged: {name}"
            );
        }
    }
}
