use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::artifact_store::{FinalizedCapture, discover_sessions, remove_session_artifacts};
use super::implementation::{
    CaptureReclamationReport, CaptureRecoveryReport, CaptureSessionOutcome, CaptureStoreError,
    CaptureStoreResult,
};
use crate::{ArtifactRepository, CaptureRetentionTracker, CaptureSessionId};

#[derive(Clone)]
pub struct CaptureSessionRepositoryConfig {
    repository: Arc<dyn ArtifactRepository>,
    max_recent_sessions: usize,
    max_total_bytes: u64,
}

impl CaptureSessionRepositoryConfig {
    pub fn new(repository: Arc<dyn ArtifactRepository>) -> Self {
        Self {
            repository,
            max_recent_sessions: 10,
            max_total_bytes: 20 * 1024 * 1024 * 1024,
        }
    }

    pub fn with_limits(
        mut self,
        max_recent_sessions: usize,
        max_total_bytes: u64,
    ) -> CaptureStoreResult<Self> {
        if max_recent_sessions == 0 || max_total_bytes == 0 {
            return Err(CaptureStoreError::InvalidConfig(
                "capture-session limits must be non-zero".into(),
            ));
        }
        self.max_recent_sessions = max_recent_sessions;
        self.max_total_bytes = max_total_bytes;
        Ok(self)
    }

    pub fn repository(&self) -> Arc<dyn ArtifactRepository> {
        Arc::clone(&self.repository)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureSessionSummary {
    pub session_id: CaptureSessionId,
    pub outcome: CaptureSessionOutcome,
    pub created_unix_ns: u64,
    pub accessed_unix_ns: u64,
    pub committed_samples: u64,
    pub bytes: u64,
    pub kept: bool,
    pub recovery: CaptureRecoveryReport,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureSessionCleanupPlan {
    pub total_sessions: usize,
    pub total_bytes: u64,
    pub over_session_limit: usize,
    pub over_byte_limit: u64,
    pub discard_candidates: Vec<CaptureSessionId>,
}

#[derive(Default)]
struct RepositoryPins {
    sessions: HashMap<CaptureSessionId, usize>,
}

#[derive(Clone)]
pub struct CaptureSessionRepository {
    config: CaptureSessionRepositoryConfig,
    pins: Arc<Mutex<RepositoryPins>>,
}

impl CaptureSessionRepository {
    pub fn new(config: CaptureSessionRepositoryConfig) -> CaptureStoreResult<Self> {
        Ok(Self {
            config,
            pins: Arc::new(Mutex::new(RepositoryPins::default())),
        })
    }

    pub fn artifact_repository(&self) -> Arc<dyn ArtifactRepository> {
        self.config.repository()
    }

    pub fn reserve(&self, session_id: CaptureSessionId) -> CaptureStoreResult<CaptureSessionPin> {
        if discover_sessions(self.config.repository.as_ref())?
            .iter()
            .any(|(existing, _)| *existing == session_id)
        {
            return Err(CaptureStoreError::InvalidConfig(format!(
                "capture session {session_id} already exists"
            )));
        }
        Ok(self.pin_unchecked(session_id))
    }

    pub fn pin(&self, session_id: CaptureSessionId) -> CaptureStoreResult<CaptureSessionPin> {
        if !discover_sessions(self.config.repository.as_ref())?
            .iter()
            .any(|(existing, _)| *existing == session_id)
        {
            return Err(CaptureStoreError::SessionNotFound(session_id));
        }
        Ok(self.pin_unchecked(session_id))
    }

    fn pin_unchecked(&self, session_id: CaptureSessionId) -> CaptureSessionPin {
        let mut pins = self.pins.lock().unwrap_or_else(|error| error.into_inner());
        *pins.sessions.entry(session_id).or_default() += 1;
        CaptureSessionPin {
            session_id,
            pins: Arc::clone(&self.pins),
        }
    }

    pub fn is_pinned(&self, session_id: CaptureSessionId) -> bool {
        self.pins
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sessions
            .get(&session_id)
            .is_some_and(|pins| *pins != 0)
    }

    pub fn scan(&self) -> CaptureStoreResult<Vec<CaptureSessionSummary>> {
        let mut summaries = Vec::new();
        for (session_id, bytes) in discover_sessions(self.config.repository.as_ref())? {
            match FinalizedCapture::recover(self.config.repository(), session_id) {
                Ok((capture, recovery)) => {
                    let manifest = capture.manifest();
                    let metadata = capture.session_metadata()?.ok_or_else(|| {
                        CaptureStoreError::Corrupt("capture session metadata is missing".into())
                    })?;
                    summaries.push(CaptureSessionSummary {
                        session_id,
                        outcome: metadata.outcome,
                        created_unix_ns: metadata.created_unix_ns,
                        accessed_unix_ns: metadata.accessed_unix_ns,
                        committed_samples: manifest.committed_samples,
                        bytes,
                        kept: metadata.kept,
                        recovery,
                        error: None,
                    });
                }
                Err(error) => summaries.push(CaptureSessionSummary {
                    session_id,
                    outcome: CaptureSessionOutcome::Corrupt,
                    created_unix_ns: 0,
                    accessed_unix_ns: 0,
                    committed_samples: 0,
                    bytes,
                    kept: false,
                    recovery: CaptureRecoveryReport::default(),
                    error: Some(error.to_string()),
                }),
            }
        }
        summaries.sort_by(|left, right| {
            right
                .accessed_unix_ns
                .cmp(&left.accessed_unix_ns)
                .then_with(|| right.created_unix_ns.cmp(&left.created_unix_ns))
        });
        Ok(summaries)
    }

    pub fn open(
        &self,
        session_id: CaptureSessionId,
    ) -> CaptureStoreResult<(FinalizedCapture, CaptureSessionPin)> {
        let pin = self.pin(session_id)?;
        let (capture, _) = FinalizedCapture::recover(self.config.repository(), session_id)?;
        capture.touch()?;
        Ok((capture, pin))
    }

    pub fn set_kept(&self, session_id: CaptureSessionId, kept: bool) -> CaptureStoreResult<()> {
        let (capture, _pin) = self.open(session_id)?;
        capture.set_kept(kept)
    }

    pub fn reclaim_to_policy(
        &self,
        session_id: CaptureSessionId,
    ) -> CaptureStoreResult<CaptureReclamationReport> {
        if self.is_pinned(session_id) {
            return Err(CaptureStoreError::SessionPinned(session_id));
        }
        let (capture, _) = FinalizedCapture::recover(self.config.repository(), session_id)?;
        let manifest = capture.manifest();
        let metadata = capture.session_metadata()?.ok_or_else(|| {
            CaptureStoreError::InvalidConfig("capture session has no retention metadata".into())
        })?;
        let plan = capture.session_plan()?.ok_or_else(|| {
            CaptureStoreError::InvalidConfig(
                "capture session has no negotiated retention policy".into(),
            )
        })?;
        let tracker = CaptureRetentionTracker::new(
            plan.sample_rate_hz,
            plan.policy.effective.retention_before_origin,
            plan.policy.effective.retention_after_origin,
        )
        .map_err(|error| CaptureStoreError::InvalidConfig(error.to_string()))?;
        let safe = tracker.safe_reclaim_before(
            manifest.committed_samples,
            manifest.committed_data_bytes,
            metadata.recording_origin,
        );
        capture.reclaim_before(safe)
    }

    pub fn discard(&self, session_id: CaptureSessionId) -> CaptureStoreResult<()> {
        if self.is_pinned(session_id) {
            return Err(CaptureStoreError::SessionPinned(session_id));
        }
        remove_session_artifacts(self.config.repository.as_ref(), session_id)
    }

    pub fn cleanup_plan(&self) -> CaptureStoreResult<CaptureSessionCleanupPlan> {
        let summaries = self.scan()?;
        Ok(self.cleanup_plan_for(&summaries))
    }

    pub fn scan_with_cleanup_plan(
        &self,
    ) -> CaptureStoreResult<(Vec<CaptureSessionSummary>, CaptureSessionCleanupPlan)> {
        let summaries = self.scan()?;
        let plan = self.cleanup_plan_for(&summaries);
        Ok((summaries, plan))
    }

    fn cleanup_plan_for(&self, summaries: &[CaptureSessionSummary]) -> CaptureSessionCleanupPlan {
        let total_sessions = summaries.len();
        let total_bytes: u64 = summaries.iter().map(|summary| summary.bytes).sum();
        let mut candidates = summaries
            .iter()
            .filter(|summary| !summary.kept && !self.is_pinned(summary.session_id))
            .map(|summary| (summary.accessed_unix_ns, summary.session_id, summary.bytes))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.0);
        let mut remaining_sessions = total_sessions;
        let mut remaining_bytes = total_bytes;
        let mut discard_candidates = Vec::new();
        for (_, session_id, bytes) in candidates {
            if remaining_sessions <= self.config.max_recent_sessions
                && remaining_bytes <= self.config.max_total_bytes
            {
                break;
            }
            discard_candidates.push(session_id);
            remaining_sessions = remaining_sessions.saturating_sub(1);
            remaining_bytes = remaining_bytes.saturating_sub(bytes);
        }
        CaptureSessionCleanupPlan {
            total_sessions,
            total_bytes,
            over_session_limit: total_sessions.saturating_sub(self.config.max_recent_sessions),
            over_byte_limit: total_bytes.saturating_sub(self.config.max_total_bytes),
            discard_candidates,
        }
    }
}

pub struct CaptureSessionPin {
    session_id: CaptureSessionId,
    pins: Arc<Mutex<RepositoryPins>>,
}

impl CaptureSessionPin {
    pub const fn session_id(&self) -> CaptureSessionId {
        self.session_id
    }
}

impl Drop for CaptureSessionPin {
    fn drop(&mut self) {
        let mut pins = self.pins.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(count) = pins.sessions.get_mut(&self.session_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                pins.sessions.remove(&self.session_id);
            }
        }
    }
}
