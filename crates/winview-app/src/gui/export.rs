// winview-app/src/gui/export.rs

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use anyhow::{ensure, Context, Result};
use eframe::egui;
use std::fs::OpenOptions;
use std::io::BufWriter;

use winview_core::model::InspectionSnapshot;

use crate::cli::inspect::write_snapshot_json;

const EXAMPLE_DESTINATION: &str = "C:\\Users\\YourName\\Documents\\winview-snapshot.json";

type ExportResult = std::result::Result<PathBuf, String>;

fn export_snapshot_json(snapshot: &InspectionSnapshot, path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "Enter an absolute destination file path"
    );

    ensure!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json")),
        "The destination must have a .json extension"
    );

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "Could not create export file: {}. \
                 Existing files are never overwritten.",
                path.display()
            )
        })?;

    write_snapshot_json(snapshot, BufWriter::new(file)).with_context(|| {
        format!(
            "Export failed while writing {}. \
             An incomplete new file may remain.",
            path.display()
        )
    })?;

    Ok(())
}

#[derive(Default)]
enum ExportState {
    #[default]
    Idle,

    Running(JoinHandle<ExportResult>),

    Finished(ExportResult),
}

impl ExportState {
    fn is_running(&self) -> bool {
        matches!(self, Self::Running(_))
    }

    fn start_with(&mut self, worker: impl FnOnce() -> ExportResult + Send + 'static) -> bool {
        if self.is_running() {
            return false;
        }

        let handle = thread::Builder::new()
            .name("winview-export".into())
            .spawn(worker);

        match handle {
            Ok(handle) => {
                *self = Self::Running(handle);
                true
            }
            Err(error) => {
                *self = Self::Finished(Err(format!("Could not start export worker: {error}")));
                false
            }
        }
    }

    fn poll(&mut self) {
        let finished = matches!(self, Self::Running(handle) if handle.is_finished());

        if !finished {
            return;
        }

        let previous = std::mem::take(self);

        let Self::Running(handle) = previous else {
            unreachable!("Only a completed worker may be collected");
        };

        *self = match handle.join() {
            Ok(result) => Self::Finished(result),
            Err(_) => Self::Finished(Err("Export worker panicked".into())),
        };
    }

    fn result(&self) -> Option<&ExportResult> {
        match self {
            Self::Finished(result) => Some(result),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct ExportBrowser {
    destination: String,
    state: ExportState,
}

impl ExportBrowser {
    pub(super) fn is_running(&self) -> bool {
        self.state.is_running()
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn start(&mut self, snapshot: Arc<InspectionSnapshot>) -> bool {
        let path = PathBuf::from(self.destination.trim());

        self.state.start_with(move || {
            export_snapshot_json(&snapshot, &path).map_err(|error| format!("{error:#}"))?;

            Ok(path)
        })
    }

    pub(super) fn poll(&mut self) {
        self.state.poll();
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, snapshot: &Arc<InspectionSnapshot>) {
        let running = self.is_running();

        egui::CollapsingHeader::new("Export captured snapshot")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(
                    "Exports contain the complete captured snapshot, \
                     including raw values and recorded diagnostics.",
                );

                ui.label(
                    "Snapshots may contain personal paths and \
                     system-specific information.",
                );

                ui.add_space(4.0);

                ui.add_enabled(
                    !running,
                    egui::TextEdit::singleline(&mut self.destination)
                        .desired_width(f32::INFINITY)
                        .hint_text(EXAMPLE_DESTINATION),
                );

                ui.small(format!("Example destination: {EXAMPLE_DESTINATION}"));

                ui.add_space(4.0);

                if ui
                    .add_enabled(!running, egui::Button::new("Export JSON"))
                    .clicked()
                {
                    self.start(Arc::clone(snapshot));
                }

                ui.add_space(4.0);

                ui.label("Existing files will not be overwritten.");

                ui.label("The exported snapshot is not a restorable registry backup.");

                ui.label("Keep the application open until the export completes.");

                ui.small(
                    "A failed or interrupted write may leave an incomplete new file. \
                     Export is not atomic.",
                );
            });

        if self.is_running() {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();

                ui.label("Exporting captured snapshot...");
            });
        }

        if let Some(result) = self.state.result() {
            match result {
                Ok(path) => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("Export complete: {}", path.display()));
                    });
                }
                Err(error) => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("Export failed: {error}"));
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use winview_core::model::{
        DecodingStatus, InspectionError, InspectionStatus, ObservationSource, RawRegistryValue,
        ReadError, RegistryValue, ValueReadResult,
    };
    use winview_core::RootKey;

    fn wait_for_completion(state: &mut ExportState) {
        let deadline = Instant::now() + Duration::from_secs(10);

        while state.is_running() {
            state.poll();

            if !state.is_running() {
                break;
            }

            assert!(Instant::now() < deadline, "Export worker timed out");

            thread::sleep(Duration::from_millis(1));
        }
    }

    fn synthetic_snapshot(status: InspectionStatus) -> InspectionSnapshot {
        let mut snapshot = InspectionSnapshot {
            status,
            ..Default::default()
        };

        snapshot.metadata.captured_at = "2026-11-26T02:51:00Z".into();

        snapshot.advanced.values.insert(
            "MaxQword".into(),
            ValueReadResult::Success(RawRegistryValue {
                key_path: "Software\\WinViewRs\\Test".into(),
                value_name: "MaxQword".into(),
                root: RootKey::Hkcu,
                raw_type_id: 11,
                raw_bytes: u64::MAX.to_le_bytes().to_vec(),
                decoded: Some(RegistryValue::Qword(u64::MAX)),
                decoding_status: DecodingStatus::Decoded,
                source: ObservationSource::HkcuAdvanced,
            }),
        );

        snapshot.advanced.values.insert(
            "MissingValue".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "MissingValue".into(),
            }),
        );

        snapshot.errors.push(InspectionError {
            key_path: "Software\\WinViewRs\\Test".into(),
            root: RootKey::Hkcu,
            operation: "open_key".into(),
            error: "injected recorded failure".into(),
        });

        snapshot
    }

    #[test]
    fn json_export_preserves_complete_snapshot() {
        let statuses = [
            InspectionStatus::Complete,
            InspectionStatus::Partial {
                reason: "injected partial".into(),
            },
            InspectionStatus::Failed {
                error: "injected failure".into(),
            },
        ];

        let directory = tempfile::tempdir().unwrap();

        for (index, status) in statuses.into_iter().enumerate() {
            let snapshot = synthetic_snapshot(status);

            let before = serde_json::to_value(&snapshot).unwrap();

            let path = directory.path().join(format!("snapshot-{index}.json"));

            export_snapshot_json(&snapshot, &path).unwrap();

            let bytes = std::fs::read(&path).unwrap();

            let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(parsed, before);

            let after = serde_json::to_value(&snapshot).unwrap();

            assert_eq!(before, after);
        }
    }

    #[test]
    fn json_export_never_overwrites_existing_files() {
        let directory = tempfile::tempdir().unwrap();

        let path = directory.path().join("existing.json");

        std::fs::write(&path, b"sentinel-bytes").unwrap();

        let snapshot = Arc::new(synthetic_snapshot(InspectionStatus::Complete));

        let mut browser = ExportBrowser {
            destination: path.display().to_string(),
            ..Default::default()
        };

        assert!(browser.start(Arc::clone(&snapshot)));

        let deadline = Instant::now() + Duration::from_secs(10);

        while browser.is_running() {
            browser.poll();

            assert!(Instant::now() < deadline, "Export worker timed out");

            thread::sleep(Duration::from_millis(1));
        }

        let error = match browser.state.result() {
            Some(Err(error)) => error.clone(),
            _ => panic!("Existing files must never be overwritten"),
        };

        assert!(error.contains("Existing files are never overwritten"));

        let underlying = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect_err("create_new must fail on an existing file");

        assert!(
            error.contains(&underlying.to_string()),
            "error chain must include the underlying filesystem error: {error}"
        );

        let bytes = std::fs::read(&path).unwrap();

        assert_eq!(bytes, b"sentinel-bytes");
    }

    #[test]
    fn json_export_rejects_invalid_destinations() {
        let directory = tempfile::tempdir().unwrap();

        let snapshot = synthetic_snapshot(InspectionStatus::Complete);

        let relative = PathBuf::from("relative-snapshot.json");

        let error = export_snapshot_json(&snapshot, &relative)
            .expect_err("Relative destinations must be rejected");

        assert!(error.to_string().contains("absolute destination"));

        let wrong_extension = directory.path().join("snapshot.txt");

        let error = export_snapshot_json(&snapshot, &wrong_extension)
            .expect_err("Non-json extensions must be rejected");

        assert!(error.to_string().contains(".json extension"));

        let missing_parent = directory.path().join("missing").join("snapshot.json");

        let error = export_snapshot_json(&snapshot, &missing_parent)
            .expect_err("Missing parent directories must produce an error");

        assert!(error.to_string().contains("Could not create export file"));

        let valid = directory.path().join("valid.json");

        export_snapshot_json(&snapshot, &valid).unwrap();

        assert!(valid.exists());
    }

    #[test]
    fn export_worker_runs_off_caller_thread() {
        let mut state = ExportState::default();

        let caller_thread = thread::current().id();

        let (started_tx, started_rx) = mpsc::channel();

        let (release_tx, release_rx) = mpsc::channel::<()>();

        assert!(state.start_with(move || {
            started_tx.send(thread::current().id()).unwrap();

            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();

            Ok(PathBuf::from("C:\\exports\\snapshot.json"))
        }));

        let worker_thread = started_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        assert_ne!(caller_thread, worker_thread);

        assert!(state.is_running());

        state.poll();

        assert!(state.is_running());

        assert!(!state.start_with(|| {
            panic!("Overlapping export must not start");
        }));

        release_tx.send(()).unwrap();

        wait_for_completion(&mut state);

        assert!(matches!(state, ExportState::Finished(_)));

        match state.result() {
            Some(Ok(path)) => assert_eq!(path, &PathBuf::from("C:\\exports\\snapshot.json")),
            _ => panic!("Successful export must report its destination"),
        }
    }

    #[test]
    fn export_worker_failure_allows_retry() {
        let mut state = ExportState::default();

        assert!(state.start_with(|| Err("injected export failure".into())));

        wait_for_completion(&mut state);

        assert!(matches!(
            state.result(),
            Some(Err(message)) if message == "injected export failure"
        ));

        assert!(state.start_with(|| {
            panic!("injected worker panic");
        }));

        wait_for_completion(&mut state);

        assert!(matches!(
            state.result(),
            Some(Err(message)) if message == "Export worker panicked"
        ));

        let snapshot = Arc::new(synthetic_snapshot(InspectionStatus::Complete));

        let directory = tempfile::tempdir().unwrap();

        let path = directory.path().join("retry.json");

        let retry_path = path.clone();

        let worker_snapshot = Arc::clone(&snapshot);

        assert!(state.start_with(move || {
            export_snapshot_json(&worker_snapshot, &retry_path)
                .map_err(|error| error.to_string())?;

            Ok(retry_path)
        }));

        wait_for_completion(&mut state);

        match state.result() {
            Some(Ok(saved)) => assert_eq!(saved, &path),
            _ => panic!("Retry export must succeed"),
        }

        assert!(path.exists());

        assert!(Arc::strong_count(&snapshot) >= 1);
    }
}
