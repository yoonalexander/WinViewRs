// winview-app/src/gui/inspection.rs

use std::sync::Arc;
use std::thread::{self, JoinHandle};

use winview_core::inspection::count_snapshot_read_errors;

use winview_core::model::InspectionSnapshot;

use super::folder_types::FolderTypesIndex;
use super::overview::InspectionOverview;

pub struct CapturedInspection {
    pub snapshot: Arc<InspectionSnapshot>,
    pub error_count: usize,
    pub overview: InspectionOverview,
    pub folder_index: FolderTypesIndex,
}

impl Clone for CapturedInspection {
    fn clone(&self) -> Self {
        Self {
            snapshot: Arc::clone(&self.snapshot),
            error_count: self.error_count,
            overview: self.overview.clone(),
            folder_index: self.folder_index.clone(),
        }
    }
}

#[derive(Default)]
pub(super) enum InspectionState {
    #[default]
    Idle,

    Loading(JoinHandle<Result<CapturedInspection, String>>),

    Finished(Box<CapturedInspection>),

    Error(String),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum InspectionStateKind {
    Idle,
    Loading,
    Finished,
    Error,
}

impl InspectionState {
    pub(super) fn kind(&self) -> InspectionStateKind {
        match self {
            Self::Idle => InspectionStateKind::Idle,
            Self::Loading(_) => InspectionStateKind::Loading,
            Self::Finished(_) => InspectionStateKind::Finished,
            Self::Error(_) => InspectionStateKind::Error,
        }
    }

    pub(super) fn is_loading(&self) -> bool {
        matches!(self, Self::Loading(_))
    }

    pub(super) fn start(
        &mut self,
        inspect: impl FnOnce() -> Result<InspectionSnapshot, String> + Send + 'static,
    ) -> bool {
        if self.is_loading() {
            return false;
        }

        let worker = thread::Builder::new()
            .name("winview-inspection".into())
            .spawn(move || {
                let snapshot = inspect()?;

                let error_count = count_snapshot_read_errors(&snapshot);

                let overview = InspectionOverview::from_snapshot(&snapshot);

                let folder_index = FolderTypesIndex::from_snapshot(&snapshot);

                Ok(CapturedInspection {
                    snapshot: Arc::new(snapshot),
                    error_count,
                    overview,
                    folder_index,
                })
            });

        match worker {
            Ok(handle) => {
                *self = Self::Loading(handle);
                true
            }
            Err(error) => {
                *self = Self::Error(format!("Could not start inspection worker: {error}"));
                false
            }
        }
    }

    pub(super) fn poll(&mut self) {
        let finished = matches!(self, Self::Loading(handle) if handle.is_finished());

        if !finished {
            return;
        }

        let previous = std::mem::take(self);

        let Self::Loading(handle) = previous else {
            unreachable!("Only a completed worker may be collected");
        };

        *self = match handle.join() {
            Ok(Ok(result)) => Self::Finished(Box::new(result)),
            Ok(Err(error)) => Self::Error(error),
            Err(_) => Self::Error("Inspection worker panicked".into()),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use winview_core::model::{InspectionError, InspectionStatus};

    use winview_core::RootKey;

    fn wait_for_completion(state: &mut InspectionState) {
        let deadline = Instant::now() + Duration::from_secs(10);

        while state.is_loading() {
            state.poll();

            if !state.is_loading() {
                break;
            }

            assert!(Instant::now() < deadline, "Inspection worker timed out");

            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn inspection_state_starts_idle() {
        let mut state = InspectionState::default();

        assert!(matches!(state, InspectionState::Idle));

        assert!(!state.is_loading());

        state.poll();

        assert!(matches!(state, InspectionState::Idle));
    }

    #[test]
    fn inspection_runs_off_caller_thread_and_rejects_overlap() {
        let mut state = InspectionState::default();

        let caller_thread = thread::current().id();

        let (started_tx, started_rx) = mpsc::channel();

        let (release_tx, release_rx) = mpsc::channel::<()>();

        assert!(state.start(move || {
            started_tx.send(thread::current().id()).unwrap();

            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();

            Ok(InspectionSnapshot::default())
        }));

        let worker_thread = started_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        assert_ne!(caller_thread, worker_thread);

        assert!(state.is_loading());

        state.poll();

        assert!(state.is_loading());

        assert!(!state.start(|| {
            panic!("Overlapping worker must not start");
        }));

        release_tx.send(()).unwrap();

        wait_for_completion(&mut state);

        assert!(matches!(state, InspectionState::Finished(_)));
    }

    #[test]
    fn inspection_preserves_all_snapshot_statuses() {
        let cases = [
            InspectionStatus::Complete,
            InspectionStatus::Partial {
                reason: "injected partial".into(),
            },
            InspectionStatus::Failed {
                error: "injected failure".into(),
            },
        ];

        let mut state = InspectionState::default();

        for status in cases {
            let expected = status.clone();

            let mut snapshot = InspectionSnapshot {
                status,
                ..Default::default()
            };

            snapshot.metadata.captured_at = "synthetic timestamp".into();

            let expected_errors = if matches!(&expected, InspectionStatus::Complete) {
                0
            } else {
                snapshot.errors.push(InspectionError {
                    key_path: "Software\\Test".into(),
                    root: RootKey::Hkcu,
                    operation: "open_key".into(),
                    error: "injected error".into(),
                });

                1
            };

            assert!(state.start(move || Ok(snapshot)));

            wait_for_completion(&mut state);

            let InspectionState::Finished(result) = &state else {
                panic!("Returned snapshot must be retained");
            };

            assert_eq!(result.snapshot.status, expected);

            assert_eq!(result.snapshot.metadata.captured_at, "synthetic timestamp");

            assert_eq!(result.error_count, expected_errors);

            assert_eq!(result.overview.metadata[4].1, "synthetic timestamp");

            assert_eq!(result.overview.global_errors, expected_errors);
        }
    }

    #[test]
    fn inspection_operational_failure_allows_retry() {
        let mut state = InspectionState::default();

        assert!(state.start(|| Err("injected operational failure".into())));

        wait_for_completion(&mut state);

        assert!(matches!(
            &state,
            InspectionState::Error(message) if message == "injected operational failure"
        ));

        assert!(state.start(|| Ok(InspectionSnapshot::default())));

        wait_for_completion(&mut state);

        assert!(matches!(state, InspectionState::Finished(_)));
    }

    #[test]
    fn inspection_worker_panic_allows_retry() {
        let mut state = InspectionState::default();

        assert!(state.start(|| {
            panic!("injected worker panic");
        }));

        wait_for_completion(&mut state);

        assert!(matches!(
            &state,
            InspectionState::Error(message) if message == "Inspection worker panicked"
        ));

        assert!(state.start(|| Ok(InspectionSnapshot::default())));

        wait_for_completion(&mut state);

        assert!(matches!(state, InspectionState::Finished(_)));
    }
}
