use async_trait::async_trait;
use chrono::{DateTime, Utc};
use iran_split_config::{ClientId, PresetId};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};
use thiserror::Error;
use tokio::sync::{mpsc, watch, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StackPhase {
    #[default]
    Uninitialized,
    Stopped,
    StartingClient,
    PreparingRuntime,
    ValidatingConfig,
    StartingCore,
    CheckingReadiness,
    Running,
    Paused,
    Degraded,
    Stopping,
    Recovering,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ComponentPhase {
    #[default]
    Unknown,
    Checking,
    Stopped,
    Starting,
    Running,
    Degraded,
    Unavailable,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentStatus {
    pub phase: ComponentPhase,
    pub message: Option<String>,
    pub since: DateTime<Utc>,
}

impl ComponentStatus {
    #[must_use]
    pub fn new(phase: ComponentPhase, message: Option<String>) -> Self {
        Self {
            phase,
            message,
            since: Utc::now(),
        }
    }
}

impl Default for ComponentStatus {
    fn default() -> Self {
        Self::new(
            ComponentPhase::Checking,
            Some("Inspecting current status".into()),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProviderSummary {
    pub ready: u32,
    pub total: u32,
    pub rules_loaded: u64,
    pub last_refresh: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    #[default]
    ExternalHiddify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    HiddifyNotFound,
    HiddifyPortBusy,
    HiddifyEgressUnavailable,
    ConfigInvalid,
    HelperUnavailable,
    HelperUnauthorized,
    MihomoNotFound,
    MihomoStartFailed,
    ControllerTimeout,
    ProviderNotReady,
    TunCleanupFailed,
    RouteTestFailed,
    UpdateSignatureInvalid,
    OperationInProgress,
    OperationTimeout,
    OperationCancelled,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "value", rename_all = "snake_case")]
pub enum Remediation {
    Retry,
    OpenSettings,
    InstallHelper,
    ChooseHiddifyExecutable,
    InstallDependency,
    RunDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message_key: String,
    pub retryable: bool,
    pub remediation: Option<Remediation>,
    pub technical_details: Option<String>,
    pub correlation_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleBusy {
    Connecting,
    Disconnecting,
    Pausing,
    Resuming,
    Reconciling,
    ApplyingRules,
}

impl LifecycleBusy {
    const fn as_kind(self) -> OperationKind {
        match self {
            Self::Reconciling => OperationKind::Reconcile,
            Self::Connecting => OperationKind::Start,
            Self::Disconnecting => OperationKind::Stop,
            Self::Pausing => OperationKind::Pause,
            Self::Resuming => OperationKind::Resume,
            Self::ApplyingRules => OperationKind::ApplyRules,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStage {
    Preparing,
    StartingClient,
    PreparingRuntime,
    ValidatingConfig,
    StartingCore,
    CheckingReadiness,
    StoppingCore,
    StoppingProxy,
    CleaningUp,
    Recovering,
}

impl OperationStage {
    const fn from_phase(phase: StackPhase) -> Option<Self> {
        match phase {
            StackPhase::StartingClient => Some(Self::StartingClient),
            StackPhase::PreparingRuntime => Some(Self::PreparingRuntime),
            StackPhase::ValidatingConfig => Some(Self::ValidatingConfig),
            StackPhase::StartingCore => Some(Self::StartingCore),
            StackPhase::CheckingReadiness => Some(Self::CheckingReadiness),
            StackPhase::Recovering => Some(Self::Recovering),
            StackPhase::Uninitialized
            | StackPhase::Stopped
            | StackPhase::Running
            | StackPhase::Paused
            | StackPhase::Degraded
            | StackPhase::Stopping
            | StackPhase::Error => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationClient {
    pub preset: PresetId,
    pub client_id: ClientId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientComponentStatus {
    pub id: ClientId,
    pub preset: PresetId,
    pub enabled: bool,
    pub status: ComponentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackSnapshot {
    pub revision: u64,
    pub phase: StackPhase,
    #[serde(default)]
    pub busy: Option<LifecycleBusy>,
    #[serde(default)]
    pub operation_stage: Option<OperationStage>,
    #[serde(default)]
    pub operation_client: Option<OperationClient>,
    pub operation_id: Option<Uuid>,
    pub helper: ComponentStatus,
    #[serde(default)]
    pub clients: Vec<ClientComponentStatus>,
    pub mihomo: ComponentStatus,
    pub tun: ComponentStatus,
    pub dns: ComponentStatus,
    pub providers: ProviderSummary,
    pub exit_ip: Option<String>,
    pub backend: BackendKind,
    pub last_error: Option<AppError>,
    pub updated_at: DateTime<Utc>,
}

impl Default for StackSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: StackPhase::Uninitialized,
            busy: None,
            operation_stage: None,
            operation_client: None,
            operation_id: None,
            helper: ComponentStatus::default(),
            clients: Vec::new(),
            mihomo: ComponentStatus::default(),
            tun: ComponentStatus::default(),
            dns: ComponentStatus::default(),
            providers: ProviderSummary::default(),
            exit_ip: None,
            backend: BackendKind::default(),
            last_error: None,
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationAccepted {
    pub operation_id: Uuid,
    pub already_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeGeneration {
    pub generation_id: Uuid,
    pub config_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HelperStatus {
    pub available: bool,
    pub authorized: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessStatus {
    pub running: bool,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TunStatus {
    pub active: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CleanupReport {
    pub process_stopped: bool,
    pub tun_removed: bool,
    pub dns_restored: bool,
    pub routes_removed: u32,
    pub warnings: Vec<String>,
}

impl CleanupReport {
    #[must_use]
    pub fn clean(&self) -> bool {
        self.process_stopped && self.tun_removed && self.dns_restored && self.warnings.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReadinessReport {
    pub controller_ready: bool,
    pub egress_ready: bool,
    pub providers: ProviderSummary,
    pub exit_ip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeHealth {
    pub helper: ComponentStatus,
    pub clients: Vec<ClientComponentStatus>,
    pub mihomo: ComponentStatus,
    pub tun: ComponentStatus,
    pub dns: ComponentStatus,
    pub providers: ProviderSummary,
}

impl ReadinessReport {
    #[must_use]
    pub fn ready(&self) -> bool {
        self.controller_ready
            && self.egress_ready
            && self.providers.total > 0
            && self.providers.ready == self.providers.total
    }
}

#[derive(Debug, Error, Clone)]
pub enum CoreError {
    #[error("helper is unavailable")]
    HelperUnavailable,
    #[error("helper rejected the current user")]
    HelperUnauthorized,
    #[error("Hiddify executable was not found")]
    HiddifyNotFound,
    #[error("Hiddify egress did not become ready")]
    HiddifyEgressUnavailable,
    #[error("runtime configuration is invalid: {0}")]
    ConfigInvalid(String),
    #[error("Mihomo executable was not found")]
    MihomoNotFound,
    #[error("Mihomo failed to start: {0}")]
    MihomoStartFailed(String),
    #[error("Mihomo controller readiness timed out")]
    ControllerTimeout,
    #[error("rule providers are not ready")]
    ProviderNotReady,
    #[error("owned TUN or route state could not be cleaned: {0}")]
    TunCleanupFailed(String),
    #[error("operation was cancelled")]
    Cancelled,
    #[error("another connection operation is already in progress")]
    OperationInProgress,
    #[error("operation timed out")]
    OperationTimeout,
    #[error("operation queue is unavailable")]
    QueueUnavailable,
    #[error("platform operation failed: {0}")]
    Platform(String),
}

impl CoreError {
    #[must_use]
    pub fn to_app_error(&self, correlation_id: Uuid) -> AppError {
        let (code, key, retryable, remediation) = match self {
            Self::HelperUnavailable => (
                ErrorCode::HelperUnavailable,
                "errors.helperUnavailable",
                true,
                Some(Remediation::InstallHelper),
            ),
            Self::HelperUnauthorized => (
                ErrorCode::HelperUnauthorized,
                "errors.helperUnauthorized",
                false,
                Some(Remediation::InstallHelper),
            ),
            Self::HiddifyNotFound => (
                ErrorCode::HiddifyNotFound,
                "errors.hiddifyNotFound",
                false,
                Some(Remediation::InstallDependency),
            ),
            Self::HiddifyEgressUnavailable => (
                ErrorCode::HiddifyEgressUnavailable,
                "errors.hiddifyEgressUnavailable",
                true,
                Some(Remediation::RunDiagnostics),
            ),
            Self::ConfigInvalid(_) => (
                ErrorCode::ConfigInvalid,
                "errors.configInvalid",
                false,
                Some(Remediation::OpenSettings),
            ),
            Self::MihomoNotFound => (
                ErrorCode::MihomoNotFound,
                "errors.mihomoNotFound",
                false,
                Some(Remediation::InstallDependency),
            ),
            Self::MihomoStartFailed(_) => (
                ErrorCode::MihomoStartFailed,
                "errors.mihomoStartFailed",
                true,
                Some(Remediation::RunDiagnostics),
            ),
            Self::ControllerTimeout => (
                ErrorCode::ControllerTimeout,
                "errors.controllerTimeout",
                true,
                Some(Remediation::Retry),
            ),
            Self::ProviderNotReady => (
                ErrorCode::ProviderNotReady,
                "errors.providerNotReady",
                true,
                Some(Remediation::Retry),
            ),
            Self::TunCleanupFailed(_) => (
                ErrorCode::TunCleanupFailed,
                "errors.tunCleanupFailed",
                true,
                Some(Remediation::RunDiagnostics),
            ),
            Self::Cancelled => (
                ErrorCode::OperationCancelled,
                "errors.operationCancelled",
                true,
                Some(Remediation::Retry),
            ),
            Self::OperationInProgress => (
                ErrorCode::OperationInProgress,
                "errors.operationInProgress",
                true,
                Some(Remediation::Retry),
            ),
            Self::OperationTimeout => (
                ErrorCode::OperationTimeout,
                "errors.operationTimeout",
                true,
                Some(Remediation::Retry),
            ),
            Self::QueueUnavailable | Self::Platform(_) => (
                ErrorCode::Internal,
                "errors.internal",
                true,
                Some(Remediation::RunDiagnostics),
            ),
        };
        AppError {
            code,
            message_key: key.into(),
            retryable,
            remediation,
            technical_details: Some(self.to_string()),
            correlation_id,
        }
    }
}

#[async_trait]
pub trait PlatformBackend: Send + Sync + 'static {
    async fn runtime_health(&self) -> RuntimeHealth;
    async fn helper_status(&self) -> Result<HelperStatus, CoreError>;
    async fn ensure_hiddify(&self, cancel: CancellationToken) -> Result<(), CoreError>;
    async fn ensure_clients(&self, cancel: CancellationToken) -> Result<(), CoreError> {
        self.ensure_hiddify(cancel).await
    }
    async fn prepare_runtime(&self) -> Result<RuntimeGeneration, CoreError>;
    async fn validate_runtime(&self, generation: &RuntimeGeneration) -> Result<(), CoreError>;
    async fn start_core(&self, generation: &RuntimeGeneration) -> Result<(), CoreError>;
    async fn stop_core(&self) -> Result<(), CoreError>;
    async fn stop_user_proxy(&self) -> Result<(), CoreError> {
        Ok(())
    }
    /// Clears the desktop system proxy when it points at Hiddify.
    ///
    /// Returns `true` when a Hiddify proxy was found and cleared. A system
    /// proxy at Hiddify sends browser traffic straight into the tunnel over
    /// loopback, bypassing the TUN split routing, so DIRECT domains break.
    async fn clear_hiddify_system_proxy(&self) -> Result<bool, CoreError>;
    async fn restore_hiddify_system_proxy(&self) -> Result<(), CoreError>;
    async fn core_process(&self) -> Result<ProcessStatus, CoreError>;
    async fn tun_status(&self) -> Result<TunStatus, CoreError>;
    async fn check_readiness(
        &self,
        cancel: CancellationToken,
    ) -> Result<ReadinessReport, CoreError>;
    async fn cleanup_owned_state(&self) -> Result<CleanupReport, CoreError>;
    async fn verify_not_intercepting(&self) -> Result<(), CoreError> {
        let tun = self.tun_status().await?;
        if tun.active {
            return Err(CoreError::TunCleanupFailed(
                "TUN is still active after cleanup".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum OperationKind {
    Reconcile,
    Start,
    Stop,
    Pause,
    Resume,
    ApplyRules,
}

impl OperationKind {
    const fn busy(self) -> LifecycleBusy {
        match self {
            Self::Reconcile => LifecycleBusy::Reconciling,
            Self::Start => LifecycleBusy::Connecting,
            Self::Stop => LifecycleBusy::Disconnecting,
            Self::Pause => LifecycleBusy::Pausing,
            Self::Resume => LifecycleBusy::Resuming,
            Self::ApplyRules => LifecycleBusy::ApplyingRules,
        }
    }

    const fn initial_stage(self) -> OperationStage {
        match self {
            Self::Reconcile => OperationStage::Recovering,
            Self::Start | Self::Resume | Self::ApplyRules => OperationStage::Preparing,
            Self::Stop | Self::Pause => OperationStage::StoppingCore,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct OperationTimeouts {
    start: Duration,
    stop: Duration,
}

impl Default for OperationTimeouts {
    fn default() -> Self {
        Self {
            start: Duration::from_secs(120),
            stop: Duration::from_secs(45),
        }
    }
}

#[derive(Debug, Clone)]
struct OperationRecord {
    cancel: CancellationToken,
}

#[derive(Debug)]
struct WorkItem {
    id: Uuid,
    kind: OperationKind,
    cancel: CancellationToken,
}

pub struct Engine<B: PlatformBackend> {
    backend: Arc<B>,
    snapshots: watch::Sender<StackSnapshot>,
    queue: mpsc::Sender<WorkItem>,
    operations: Mutex<HashMap<Uuid, OperationRecord>>,
    pending: Mutex<HashMap<OperationKind, Uuid>>,
    timeouts: OperationTimeouts,
}

impl<B: PlatformBackend> std::fmt::Debug for Engine<B> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Engine")
            .field("snapshot", &*self.snapshots.borrow())
            .finish_non_exhaustive()
    }
}

impl<B: PlatformBackend> Engine<B> {
    /// Creates an engine and schedules its worker on `runtime`.
    ///
    /// Passing the runtime explicitly allows GUI setup code to construct the
    /// engine outside an entered Tokio context without panicking.
    #[must_use]
    pub fn new(backend: Arc<B>, runtime: &tokio::runtime::Handle) -> Arc<Self> {
        Self::construct(backend, runtime, OperationTimeouts::default())
    }

    fn construct(
        backend: Arc<B>,
        runtime: &tokio::runtime::Handle,
        timeouts: OperationTimeouts,
    ) -> Arc<Self> {
        let (queue, receiver) = mpsc::channel(16);
        let (snapshots, _) = watch::channel(StackSnapshot::default());
        let engine = Arc::new(Self {
            backend,
            snapshots,
            queue,
            operations: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            timeouts,
        });
        runtime.spawn(Self::worker(Arc::clone(&engine), receiver));
        engine
    }

    #[cfg(test)]
    #[must_use]
    pub fn new_with_timeouts(
        backend: Arc<B>,
        runtime: &tokio::runtime::Handle,
        start: Duration,
        stop: Duration,
    ) -> Arc<Self> {
        Self::construct(backend, runtime, OperationTimeouts { start, stop })
    }

    #[must_use]
    pub fn snapshot(&self) -> StackSnapshot {
        self.snapshots.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<StackSnapshot> {
        self.snapshots.subscribe()
    }

    pub async fn refresh_health(&self) {
        info!(
            event = "health.refresh_started",
            section = "runtime_health",
            initiator = "engine",
            cause = "scheduled_or_requested",
            trace_route = "engine->platform_backend->runtime_health",
            "runtime health refresh started"
        );
        let health = self.backend.runtime_health().await;
        self.update(|snapshot| apply_health(snapshot, health));
        // Hiddify re-asserts the desktop system proxy whenever it reconnects.
        // While the stack is up that proxy bypasses the TUN split routing and
        // silently breaks DIRECT domains, so keep enforcing its removal and
        // leave an evidence trail when it happens.
        if matches!(
            self.snapshot().phase,
            StackPhase::Running | StackPhase::Degraded
        ) {
            match self.backend.clear_hiddify_system_proxy().await {
                Ok(true) => warn!(
                    event = "system_proxy.hijack_cleared",
                    section = "system_proxy",
                    initiator = "engine",
                    cause = "hiddify_reasserted_system_proxy",
                    trace_route = "engine->platform_backend->clear_hiddify_system_proxy",
                    "system proxy pointed at Hiddify while running; cleared it so DIRECT domains keep working"
                ),
                Ok(false) => {}
                Err(cause) => info!(
                    event = "system_proxy.enforce_failed",
                    section = "system_proxy",
                    initiator = "engine",
                    cause = %cause,
                    trace_route = "engine->platform_backend->clear_hiddify_system_proxy",
                    "could not verify the desktop system proxy during health refresh"
                ),
            }
        }
        info!(
            event = "health.refresh_completed",
            section = "runtime_health",
            initiator = "engine",
            cause = "none",
            trace_route = "engine->platform_backend->runtime_health",
            "runtime health refresh completed"
        );
    }

    /// Queues startup reconciliation of helper and network state.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::QueueUnavailable`] when the operation worker is no
    /// longer available.
    pub async fn reconcile_startup(&self) -> Result<OperationAccepted, CoreError> {
        self.accept(OperationKind::Reconcile).await
    }

    /// Queues a stack start, or reports success when already running.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::QueueUnavailable`] when the operation worker is no
    /// longer available.
    pub async fn start_stack(&self) -> Result<OperationAccepted, CoreError> {
        if self.snapshot().busy.is_some() && self.snapshot().busy != Some(LifecycleBusy::Connecting)
        {
            return Err(CoreError::OperationInProgress);
        }
        if self.snapshot().phase == StackPhase::Running {
            self.release_if_idle(OperationKind::Start).await;
            return Ok(OperationAccepted {
                operation_id: Uuid::new_v4(),
                already_complete: true,
            });
        }
        self.accept(OperationKind::Start).await
    }

    /// Cancels an in-progress start and queues a stack stop.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::QueueUnavailable`] when the operation worker is no
    /// longer available.
    pub async fn stop_stack(&self) -> Result<OperationAccepted, CoreError> {
        if self.snapshot().busy.is_some()
            && self.snapshot().busy != Some(LifecycleBusy::Disconnecting)
        {
            return Err(CoreError::OperationInProgress);
        }
        if self.snapshot().phase == StackPhase::Stopped && self.operations.lock().await.is_empty() {
            return Ok(OperationAccepted {
                operation_id: Uuid::new_v4(),
                already_complete: true,
            });
        }
        self.accept(OperationKind::Stop).await
    }

    /// Detaches owned TUN, routes, and DNS while leaving Hiddify running.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::QueueUnavailable`] when the operation worker is no
    /// longer available.
    pub async fn pause_stack(&self) -> Result<OperationAccepted, CoreError> {
        if self.snapshot().busy.is_some() && self.snapshot().busy != Some(LifecycleBusy::Pausing) {
            return Err(CoreError::OperationInProgress);
        }
        if matches!(
            self.snapshot().phase,
            StackPhase::Paused | StackPhase::Stopped | StackPhase::Uninitialized
        ) && self.operations.lock().await.is_empty()
        {
            return Ok(OperationAccepted {
                operation_id: Uuid::new_v4(),
                already_complete: true,
            });
        }
        self.accept(OperationKind::Pause).await
    }

    /// Rebuilds owned routing from the paused state.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::QueueUnavailable`] when the operation worker is no
    /// longer available.
    pub async fn resume_stack(&self) -> Result<OperationAccepted, CoreError> {
        if self.snapshot().busy.is_some() && self.snapshot().busy != Some(LifecycleBusy::Resuming) {
            return Err(CoreError::OperationInProgress);
        }
        if self.snapshot().phase == StackPhase::Running {
            return Ok(OperationAccepted {
                operation_id: Uuid::new_v4(),
                already_complete: true,
            });
        }
        if self.snapshot().phase != StackPhase::Paused {
            return Ok(OperationAccepted {
                operation_id: Uuid::new_v4(),
                already_complete: true,
            });
        }
        self.accept(OperationKind::Resume).await
    }

    /// Regenerates and reloads Mihomo rule providers while the stack is live.
    ///
    /// Stopped or paused stacks only need the persisted document; the next
    /// start consumes that revision. A conflicting lifecycle lock is retryable.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::OperationInProgress`] when Connect/Disconnect/Pause
    /// is reserved, or a platform/readiness error when the live reload fails.
    pub async fn apply_user_rules(&self) -> Result<(), CoreError> {
        if !matches!(
            self.snapshot().phase,
            StackPhase::Running | StackPhase::Degraded
        ) {
            return Ok(());
        }
        self.reserve_lifecycle(LifecycleBusy::ApplyingRules).await?;
        let cancel = CancellationToken::new();
        let result = tokio::time::timeout(Duration::from_secs(5), self.run_apply_rules(&cancel))
            .await
            .unwrap_or(Err(CoreError::OperationTimeout));
        self.release_lifecycle(LifecycleBusy::ApplyingRules).await;
        result
    }

    async fn run_apply_rules(&self, cancel: &CancellationToken) -> Result<(), CoreError> {
        info!(
            event = "rules.apply_started",
            section = "rules",
            initiator = "engine",
            cause = "user_pin",
            trace_route = "tauri_command->engine->apply_user_rules",
            "applying persisted pins to the live runtime generation"
        );
        let generation = self.backend.prepare_runtime().await?;
        check_cancelled(cancel)?;
        self.backend.validate_runtime(&generation).await?;
        check_cancelled(cancel)?;
        self.backend.start_core(&generation).await?;
        check_cancelled(cancel)?;
        let readiness = self.backend.check_readiness(cancel.clone()).await?;
        if !readiness.controller_ready {
            return Err(CoreError::ControllerTimeout);
        }
        if readiness.providers.total == 0 || readiness.providers.ready != readiness.providers.total
        {
            return Err(CoreError::ProviderNotReady);
        }
        self.update(|snapshot| {
            snapshot.providers = readiness.providers;
        });
        info!(
            event = "rules.apply_succeeded",
            section = "rules",
            initiator = "engine",
            cause = "providers_ready",
            trace_route = "engine->apply_user_rules->readiness",
            "live rule providers are ready"
        );
        Ok(())
    }

    /// Reserves the shared lifecycle lock before a long prepare step.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::OperationInProgress`] when another connection
    /// operation is already reserved or queued.
    pub async fn reserve_lifecycle(&self, busy: LifecycleBusy) -> Result<(), CoreError> {
        self.reserve(busy.as_kind()).await
    }

    /// Clears a reservation that never reached the worker queue.
    pub async fn release_lifecycle(&self, busy: LifecycleBusy) {
        self.release_if_idle(busy.as_kind()).await;
    }

    /// Queues a stop followed by a start.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::QueueUnavailable`] when either operation cannot be
    /// queued.
    pub async fn restart_stack(&self) -> Result<(OperationAccepted, OperationAccepted), CoreError> {
        let stop = self.stop_stack().await?;
        let start = self.accept(OperationKind::Start).await?;
        Ok((stop, start))
    }

    pub async fn cancel_operation(&self, operation_id: Uuid) -> bool {
        let operations = self.operations.lock().await;
        if let Some(operation) = operations.get(&operation_id) {
            operation.cancel.cancel();
            true
        } else {
            false
        }
    }

    async fn reserve(&self, kind: OperationKind) -> Result<(), CoreError> {
        let pending = self.pending.lock().await;
        if let Some(operation_id) = pending.get(&kind) {
            info!(
                event = "operation.deduplicated",
                section = "engine",
                initiator = "operation_queue",
                cause = "matching_operation_pending",
                trace_route = "caller->engine->reserve",
                operation_id = %operation_id,
                kind = ?kind,
                "existing operation reservation reused"
            );
            return Ok(());
        }
        if !pending.is_empty() {
            return Err(CoreError::OperationInProgress);
        }
        let current = self.snapshot().busy;
        if let Some(busy) = current {
            if busy != kind.busy() {
                return Err(CoreError::OperationInProgress);
            }
            return Ok(());
        }
        drop(pending);
        self.update(|snapshot| {
            snapshot.busy = Some(kind.busy());
            snapshot.operation_stage = Some(kind.initial_stage());
        });
        info!(
            event = "operation.reserved",
            section = "engine",
            initiator = "operation_queue",
            cause = "accepted",
            trace_route = "caller->engine->reserve",
            kind = ?kind,
            busy = ?kind.busy(),
            "lifecycle lock reserved"
        );
        Ok(())
    }

    async fn release_if_idle(&self, kind: OperationKind) {
        if !self.pending.lock().await.is_empty() {
            return;
        }
        self.update(|snapshot| {
            if snapshot.busy == Some(kind.busy()) && snapshot.operation_id.is_none() {
                snapshot.busy = None;
                snapshot.operation_stage = None;
            }
        });
    }

    fn timeout_for(&self, kind: OperationKind) -> Duration {
        match kind {
            OperationKind::Stop | OperationKind::Pause => self.timeouts.stop,
            OperationKind::Reconcile | OperationKind::Start | OperationKind::Resume => {
                self.timeouts.start
            }
            OperationKind::ApplyRules => Duration::from_secs(5),
        }
    }

    async fn execute_item(&self, item: &WorkItem) -> Result<(), CoreError> {
        let work = async {
            match item.kind {
                OperationKind::Reconcile => self.run_reconcile(item.id, &item.cancel).await,
                OperationKind::Start => self.run_start(item.id, &item.cancel).await,
                OperationKind::Stop => self.run_stop(item.id).await,
                OperationKind::Pause => self.run_pause(item.id).await,
                OperationKind::Resume => self.run_resume(item.id, &item.cancel).await,
                OperationKind::ApplyRules => self.run_apply_rules(&item.cancel).await,
            }
        };
        if let Ok(result) = tokio::time::timeout(self.timeout_for(item.kind), work).await {
            result
        } else {
            item.cancel.cancel();
            warn!(
                event = "operation.timed_out",
                section = "engine",
                initiator = "operation_worker",
                cause = "timeout",
                trace_id = %item.id,
                trace_route = "operation_queue->worker->timeout",
                operation_id = %item.id,
                kind = ?item.kind,
                "operation exceeded its time budget"
            );
            Err(CoreError::OperationTimeout)
        }
    }

    async fn accept(&self, kind: OperationKind) -> Result<OperationAccepted, CoreError> {
        self.reserve(kind).await?;
        let mut pending = self.pending.lock().await;
        if let Some(operation_id) = pending.get(&kind) {
            info!(
                event = "operation.deduplicated",
                section = "engine",
                initiator = "operation_queue",
                cause = "matching_operation_pending",
                trace_route = "caller->engine->operation_queue",
                operation_id = %operation_id,
                kind = ?kind,
                "existing operation returned"
            );
            return Ok(OperationAccepted {
                operation_id: *operation_id,
                already_complete: false,
            });
        }
        let operation_id = Uuid::new_v4();
        let cancel = CancellationToken::new();
        self.operations.lock().await.insert(
            operation_id,
            OperationRecord {
                cancel: cancel.clone(),
            },
        );
        pending.insert(kind, operation_id);
        if self
            .queue
            .send(WorkItem {
                id: operation_id,
                kind,
                cancel,
            })
            .await
            .is_err()
        {
            pending.remove(&kind);
            self.operations.lock().await.remove(&operation_id);
            error!(
                event = "operation.queue_failed",
                section = "engine",
                initiator = "operation_queue",
                cause = "worker_channel_closed",
                trace_route = "caller->engine->operation_queue",
                operation_id = %operation_id,
                kind = ?kind,
                "operation could not be queued"
            );
            return Err(CoreError::QueueUnavailable);
        }
        info!(
            event = "operation.queued",
            section = "engine",
            initiator = "operation_queue",
            cause = "accepted",
            trace_route = "caller->engine->operation_queue->worker",
            operation_id = %operation_id,
            kind = ?kind,
            "operation queued"
        );
        Ok(OperationAccepted {
            operation_id,
            already_complete: false,
        })
    }

    async fn worker(engine: Arc<Self>, mut receiver: mpsc::Receiver<WorkItem>) {
        while let Some(item) = receiver.recv().await {
            info!(
                event = "operation.started",
                section = "engine",
                initiator = "operation_worker",
                cause = "dequeued",
                trace_id = %item.id,
                trace_route = "operation_queue->worker->platform_backend",
                operation_id = %item.id,
                kind = ?item.kind,
                "operation started"
            );
            let result = engine.execute_item(&item).await;
            if let Err(error) = result {
                if !matches!(error, CoreError::Cancelled) {
                    error!(
                        event = "operation.failed",
                        section = "engine",
                        initiator = "operation_worker",
                        cause = %error,
                        trace_id = %item.id,
                        trace_route = "operation_queue->worker->platform_backend",
                        operation_id = %item.id,
                        kind = ?item.kind,
                        "operation failed"
                    );
                    let keep_paused = item.kind == OperationKind::Resume
                        && engine.snapshot().phase == StackPhase::Paused;
                    let health = engine.backend.runtime_health().await;
                    engine.update(|snapshot| {
                        apply_health(snapshot, health);
                        if keep_paused {
                            snapshot.phase = StackPhase::Paused;
                        } else {
                            snapshot.phase = StackPhase::Error;
                        }
                        snapshot.last_error = Some(error.to_app_error(item.id));
                    });
                }
            }
            engine.update(|snapshot| {
                snapshot.operation_id = None;
                snapshot.busy = None;
                snapshot.operation_stage = None;
            });
            engine.operations.lock().await.remove(&item.id);
            engine.pending.lock().await.remove(&item.kind);
            info!(
                event = "operation.finished",
                section = "engine",
                initiator = "operation_worker",
                cause = "worker_complete",
                trace_id = %item.id,
                trace_route = "operation_queue->worker->snapshot",
                operation_id = %item.id,
                kind = ?item.kind,
                "operation finished"
            );
        }
    }

    async fn run_reconcile(
        &self,
        operation_id: Uuid,
        cancel: &CancellationToken,
    ) -> Result<(), CoreError> {
        self.transition(StackPhase::Recovering, operation_id);
        check_cancelled(cancel)?;
        let health = self.backend.runtime_health().await;
        let helper_ready = health.helper.phase == ComponentPhase::Running;
        self.update(|snapshot| apply_health(snapshot, health));
        if helper_ready {
            let process = self.backend.core_process().await.unwrap_or_default();
            let tun = self.backend.tun_status().await.unwrap_or_default();
            if process.running || tun.active {
                warn!(
                    event = "startup.owned_state_found",
                    section = "startup_reconciliation",
                    initiator = "engine",
                    cause = "previous_session_state_remains",
                    trace_id = %operation_id,
                    trace_route = "startup->engine->platform_backend->cleanup",
                    "found owned runtime state during startup reconciliation"
                );
                let report = self.backend.cleanup_owned_state().await?;
                if !report.clean() {
                    return Err(CoreError::TunCleanupFailed(report.warnings.join("; ")));
                }
            }
        }
        let health = self.backend.runtime_health().await;
        self.set_stopped(health);
        Ok(())
    }

    async fn run_start(
        &self,
        operation_id: Uuid,
        cancel: &CancellationToken,
    ) -> Result<(), CoreError> {
        if self.snapshot().phase == StackPhase::Running {
            return Ok(());
        }
        let mut core_started = false;
        let result = self
            .start_steps(operation_id, cancel, &mut core_started)
            .await;
        if let Err(error) = result {
            if core_started || matches!(error, CoreError::Cancelled) {
                if let Err(rollback_error) = self.rollback(operation_id).await {
                    return Err(CoreError::TunCleanupFailed(format!(
                        "start failed ({error}); rollback failed ({rollback_error})"
                    )));
                }
            }
            if matches!(error, CoreError::Cancelled) {
                let health = self.backend.runtime_health().await;
                self.set_stopped(health);
            }
            return Err(error);
        }
        Ok(())
    }

    async fn start_enabled_clients(
        &self,
        operation_id: Uuid,
        cancel: &CancellationToken,
    ) -> Result<(), CoreError> {
        self.announce(StackPhase::StartingClient, operation_id)
            .await;
        self.update(|snapshot| {
            snapshot.operation_client =
                snapshot
                    .clients
                    .iter()
                    .find(|client| client.enabled)
                    .map(|client| OperationClient {
                        preset: client.preset,
                        client_id: client.id,
                    });
            for client in &mut snapshot.clients {
                if client.enabled {
                    client.status = ComponentStatus::new(ComponentPhase::Starting, None);
                }
            }
        });
        self.backend.ensure_clients(cancel.clone()).await?;
        check_cancelled(cancel)?;
        // An optional client may have failed alone; take the backend's view
        // instead of assuming every instance is running.
        let health = self.backend.runtime_health().await;
        self.update(|snapshot| {
            snapshot.clients = health.clients;
        });
        Ok(())
    }

    async fn start_steps(
        &self,
        operation_id: Uuid,
        cancel: &CancellationToken,
        core_started: &mut bool,
    ) -> Result<(), CoreError> {
        let helper = self.backend.helper_status().await?;
        if !helper.available {
            return Err(CoreError::HelperUnavailable);
        }
        if !helper.authorized {
            return Err(CoreError::HelperUnauthorized);
        }
        self.update(|snapshot| {
            snapshot.helper = ComponentStatus::new(
                ComponentPhase::Running,
                helper.version.map(|version| format!("Helper {version}")),
            );
        });

        self.start_enabled_clients(operation_id, cancel).await?;

        self.announce(StackPhase::PreparingRuntime, operation_id)
            .await;
        let generation = self.backend.prepare_runtime().await?;
        check_cancelled(cancel)?;

        self.announce(StackPhase::ValidatingConfig, operation_id)
            .await;
        self.backend.validate_runtime(&generation).await?;
        check_cancelled(cancel)?;

        self.announce(StackPhase::StartingCore, operation_id).await;
        self.update(|snapshot| {
            snapshot.mihomo = ComponentStatus::new(ComponentPhase::Starting, None);
            snapshot.tun = ComponentStatus::new(ComponentPhase::Starting, None);
            snapshot.dns = ComponentStatus::new(ComponentPhase::Starting, None);
        });
        self.backend.start_core(&generation).await?;
        *core_started = true;
        check_cancelled(cancel)?;

        self.announce(StackPhase::CheckingReadiness, operation_id)
            .await;
        let readiness = self.backend.check_readiness(cancel.clone()).await?;
        if !readiness.controller_ready {
            return Err(CoreError::ControllerTimeout);
        }
        if !readiness.egress_ready {
            return Err(CoreError::HiddifyEgressUnavailable);
        }
        if readiness.providers.total == 0 || readiness.providers.ready != readiness.providers.total
        {
            return Err(CoreError::ProviderNotReady);
        }
        let tun = self.confirm_core_and_tun(cancel).await?;
        // Hiddify sets the desktop system proxy to itself when it starts.
        // While the stack runs, that proxy short-circuits browsers straight
        // into the tunnel over loopback (TUN never sees the traffic), so
        // DIRECT domains break. Clear it once the stack is up; the health
        // refresh keeps enforcing it afterwards.
        match self.backend.clear_hiddify_system_proxy().await {
            Ok(true) => info!(
                event = "system_proxy.cleared_on_start",
                section = "system_proxy",
                initiator = "start_steps",
                cause = "hiddify_endpoint",
                trace_id = %operation_id,
                trace_route = "engine_operation->platform_backend->clear_hiddify_system_proxy",
                "cleared a Hiddify system proxy so browsers use the TUN split routing"
            ),
            Ok(false) => {}
            Err(cause) => warn!(
                event = "operation.start_clear_proxy_failed",
                section = "engine",
                initiator = "start_steps",
                cause = %cause,
                trace_id = %operation_id,
                trace_route = "engine_operation->platform_backend->clear_hiddify_system_proxy",
                "start continued after system proxy clear failure"
            ),
        }
        self.update(|snapshot| {
            snapshot.phase = StackPhase::Running;
            snapshot.operation_id = Some(operation_id);
            snapshot.mihomo = ComponentStatus::new(ComponentPhase::Running, None);
            snapshot.tun = ComponentStatus::new(ComponentPhase::Running, tun.name);
            snapshot.dns = ComponentStatus::new(ComponentPhase::Running, None);
            snapshot.providers = readiness.providers;
            snapshot.exit_ip = readiness.exit_ip;
            snapshot.last_error = None;
        });
        Ok(())
    }

    async fn confirm_core_and_tun(
        &self,
        cancel: &CancellationToken,
    ) -> Result<TunStatus, CoreError> {
        const BUDGET: Duration = Duration::from_secs(5);
        const INTERVAL: Duration = Duration::from_millis(200);
        let deadline = tokio::time::Instant::now() + BUDGET;
        loop {
            check_cancelled(cancel)?;
            let process = self.backend.core_process().await?;
            let tun = self.backend.tun_status().await?;
            if process.running && tun.active {
                return Ok(tun);
            }
            if tokio::time::Instant::now() >= deadline {
                error!(
                    event = "mihomo.tun_or_process_missing",
                    section = "engine",
                    initiator = "operation_worker",
                    cause = "readiness_postcondition",
                    process_running = process.running,
                    tun_active = tun.active,
                    trace_route = "operation_queue->worker->platform_backend",
                    "Mihomo process or TUN was not active after readiness"
                );
                return Err(CoreError::MihomoStartFailed(core_tun_missing_message(
                    &process, &tun,
                )));
            }
            tokio::select! {
                () = cancel.cancelled() => return Err(CoreError::Cancelled),
                () = tokio::time::sleep(INTERVAL) => {}
            }
        }
    }

    async fn rollback(&self, operation_id: Uuid) -> Result<(), CoreError> {
        self.transition(StackPhase::Recovering, operation_id);
        if let Err(cause) = self.backend.stop_core().await {
            warn!(
                event = "operation.rollback_stop_failed",
                section = "engine",
                initiator = "rollback",
                cause = %cause,
                trace_id = %operation_id,
                trace_route = "engine_operation->rollback->platform_backend",
                "rollback continued after the core stop step failed"
            );
        }
        let report = self.backend.cleanup_owned_state().await?;
        let tun = self.backend.tun_status().await?;
        if tun.active || !report.clean() {
            return Err(CoreError::TunCleanupFailed(format!(
                "cleanup warnings: {}",
                report.warnings.join("; ")
            )));
        }
        Ok(())
    }

    async fn run_stop(&self, operation_id: Uuid) -> Result<(), CoreError> {
        let was_active = matches!(
            self.snapshot().phase,
            StackPhase::Running | StackPhase::Degraded | StackPhase::Paused
        );
        self.announce(StackPhase::Stopping, operation_id).await;
        self.announce_stage(OperationStage::StoppingCore).await;
        if was_active {
            if let Err(cause) = self.backend.stop_core().await {
                warn!(
                    event = "operation.stop_core_failed",
                    section = "engine",
                    initiator = "run_stop",
                    cause = %cause,
                    trace_id = %operation_id,
                    trace_route = "engine_operation->platform_backend->stop_core",
                    "stop continued after core stop failure"
                );
            }
        }
        self.announce_stage(OperationStage::StoppingProxy).await;
        self.backend.stop_user_proxy().await?;
        self.backend.clear_hiddify_system_proxy().await?;
        self.announce_stage(OperationStage::CleaningUp).await;
        let report = self.backend.cleanup_owned_state().await?;
        let tun = self.backend.tun_status().await?;
        if tun.active || !report.clean() {
            return Err(CoreError::TunCleanupFailed(format!(
                "TUN active: {}; warnings: {}",
                tun.active,
                report.warnings.join("; ")
            )));
        }
        let health = self.backend.runtime_health().await;
        self.set_stopped(health);
        Ok(())
    }

    async fn run_pause(&self, operation_id: Uuid) -> Result<(), CoreError> {
        self.announce(StackPhase::Stopping, operation_id).await;
        self.announce_stage(OperationStage::StoppingCore).await;
        if let Err(cause) = self.backend.stop_core().await {
            warn!(
                event = "operation.pause_stop_core_failed",
                section = "engine",
                initiator = "run_pause",
                cause = %cause,
                trace_id = %operation_id,
                trace_route = "engine_operation->platform_backend->stop_core",
                "pause continued after core stop failure"
            );
        }
        if let Err(cause) = self.backend.clear_hiddify_system_proxy().await {
            warn!(
                event = "operation.pause_clear_proxy_failed",
                section = "engine",
                initiator = "run_pause",
                cause = %cause,
                trace_id = %operation_id,
                trace_route = "engine_operation->platform_backend->clear_hiddify_system_proxy",
                "pause continued after system proxy clear failure"
            );
        }
        self.announce_stage(OperationStage::CleaningUp).await;
        let report = self.backend.cleanup_owned_state().await?;
        let tun = self.backend.tun_status().await?;
        if tun.active || !report.clean() {
            return Err(CoreError::TunCleanupFailed(format!(
                "TUN active: {}; warnings: {}",
                tun.active,
                report.warnings.join("; ")
            )));
        }
        self.backend.verify_not_intercepting().await?;
        let health = self.backend.runtime_health().await;
        self.set_paused(health);
        Ok(())
    }

    async fn run_resume(
        &self,
        operation_id: Uuid,
        cancel: &CancellationToken,
    ) -> Result<(), CoreError> {
        if self.snapshot().phase == StackPhase::Running {
            return Ok(());
        }
        let mut core_started = false;
        let result = self
            .start_steps(operation_id, cancel, &mut core_started)
            .await;
        if let Err(error) = result {
            if core_started || matches!(error, CoreError::Cancelled) {
                if let Err(rollback_error) = self.rollback_pause(operation_id).await {
                    return Err(CoreError::TunCleanupFailed(format!(
                        "resume failed ({error}); rollback failed ({rollback_error})"
                    )));
                }
            }
            let health = self.backend.runtime_health().await;
            self.set_paused(health);
            return Err(error);
        }
        // Deliberately no restore_hiddify_system_proxy here: the snapshot is
        // only ever a Hiddify endpoint, and re-pointing the system proxy at
        // Hiddify while the TUN runs bypasses the split routing (the bug that
        // broke DIRECT domains after every Pause -> Resume).
        Ok(())
    }

    async fn rollback_pause(&self, operation_id: Uuid) -> Result<(), CoreError> {
        self.transition(StackPhase::Recovering, operation_id);
        if let Err(cause) = self.backend.stop_core().await {
            warn!(
                event = "operation.rollback_stop_failed",
                section = "engine",
                initiator = "rollback_pause",
                cause = %cause,
                trace_id = %operation_id,
                trace_route = "engine_operation->rollback_pause->platform_backend",
                "pause rollback continued after core stop failure"
            );
        }
        let report = self.backend.cleanup_owned_state().await?;
        let tun = self.backend.tun_status().await?;
        if tun.active || !report.clean() {
            return Err(CoreError::TunCleanupFailed(format!(
                "cleanup warnings: {}",
                report.warnings.join("; ")
            )));
        }
        Ok(())
    }

    fn transition(&self, phase: StackPhase, operation_id: Uuid) {
        self.update(|snapshot| {
            snapshot.phase = phase;
            snapshot.operation_id = Some(operation_id);
            if let Some(stage) = OperationStage::from_phase(phase) {
                snapshot.operation_stage = Some(stage);
            }
        });
    }

    fn set_operation_stage(&self, stage: OperationStage) {
        self.update(|snapshot| snapshot.operation_stage = Some(stage));
    }

    async fn announce(&self, phase: StackPhase, operation_id: Uuid) {
        self.transition(phase, operation_id);
        tokio::task::yield_now().await;
    }

    async fn announce_stage(&self, stage: OperationStage) {
        self.set_operation_stage(stage);
        tokio::task::yield_now().await;
    }

    fn set_paused(&self, health: RuntimeHealth) {
        self.update(|snapshot| {
            apply_health(snapshot, health);
            snapshot.phase = StackPhase::Paused;
            snapshot.operation_id = None;
            snapshot.operation_stage = None;
            snapshot.exit_ip = None;
            snapshot.mihomo = ComponentStatus::new(ComponentPhase::Stopped, None);
            snapshot.tun = ComponentStatus::new(ComponentPhase::Stopped, None);
            snapshot.dns = ComponentStatus::new(ComponentPhase::Stopped, None);
            snapshot.providers = ProviderSummary::default();
            snapshot.last_error = None;
        });
    }

    fn set_stopped(&self, health: RuntimeHealth) {
        self.update(|snapshot| {
            apply_health(snapshot, health);
            snapshot.phase = StackPhase::Stopped;
            snapshot.operation_id = None;
            snapshot.operation_stage = None;
            snapshot.exit_ip = None;
            snapshot.last_error = None;
        });
    }

    fn update(&self, update: impl FnOnce(&mut StackSnapshot)) {
        let mut snapshot = self.snapshots.borrow().clone();
        update(&mut snapshot);
        snapshot.revision = snapshot.revision.saturating_add(1);
        snapshot.updated_at = Utc::now();
        self.snapshots.send_replace(snapshot);
    }

    /// Waits until the stack reaches `desired`, reports an error phase, or times out.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Platform`] if the stack enters its error phase,
    /// [`CoreError::QueueUnavailable`] if snapshot delivery closes, or
    /// [`CoreError::ControllerTimeout`] when `timeout` elapses.
    pub async fn wait_for_phase(
        &self,
        desired: StackPhase,
        timeout: Duration,
    ) -> Result<StackSnapshot, CoreError> {
        let mut receiver = self.subscribe();
        tokio::time::timeout(timeout, async {
            loop {
                let snapshot = receiver.borrow().clone();
                if snapshot.phase == desired {
                    return Ok(snapshot);
                }
                if snapshot.phase == StackPhase::Error {
                    return Err(CoreError::Platform(snapshot.last_error.map_or_else(
                        || "unknown error".into(),
                        |error| error.technical_details.unwrap_or(error.message_key),
                    )));
                }
                receiver
                    .changed()
                    .await
                    .map_err(|_| CoreError::QueueUnavailable)?;
            }
        })
        .await
        .map_err(|_| CoreError::ControllerTimeout)?
    }
}

fn apply_health(snapshot: &mut StackSnapshot, health: RuntimeHealth) {
    snapshot.helper = health.helper;
    snapshot.clients = health.clients;
    snapshot.mihomo = health.mihomo;
    snapshot.tun = health.tun;
    snapshot.dns = health.dns;
    snapshot.providers = health.providers;
}

#[must_use]
fn core_tun_missing_message(process: &ProcessStatus, tun: &TunStatus) -> String {
    if process.running {
        format!(
            "TUN {} was not active after the controller was ready",
            tun.name.as_deref().unwrap_or("unknown")
        )
    } else {
        "Mihomo process disappeared during readiness checks".into()
    }
}

fn check_cancelled(cancel: &CancellationToken) -> Result<(), CoreError> {
    if cancel.is_cancelled() {
        Err(CoreError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Debug, Default)]
    struct FakeBackend {
        process: AtomicBool,
        tun: AtomicBool,
        fail_readiness: AtomicBool,
        slow_hiddify: AtomicBool,
        hiddify_missing: AtomicBool,
        helper_missing: AtomicBool,
        starts: AtomicUsize,
        cleanups: AtomicUsize,
        proxy_stops: AtomicUsize,
        proxy_clears: AtomicUsize,
        proxy_restores: AtomicUsize,
    }

    #[async_trait]
    impl PlatformBackend for FakeBackend {
        async fn runtime_health(&self) -> RuntimeHealth {
            let helper = if self.helper_missing.load(Ordering::SeqCst) {
                ComponentStatus::new(ComponentPhase::Unavailable, Some("Helper missing".into()))
            } else {
                ComponentStatus::new(ComponentPhase::Running, Some("Helper test".into()))
            };
            let hiddify = if self.hiddify_missing.load(Ordering::SeqCst) {
                ComponentStatus::new(ComponentPhase::Unavailable, Some("Hiddify missing".into()))
            } else {
                ComponentStatus::new(ComponentPhase::Running, Some("Hiddify ready".into()))
            };
            let running = self.process.load(Ordering::SeqCst);
            let tun = self.tun.load(Ordering::SeqCst);
            RuntimeHealth {
                helper,
                clients: vec![ClientComponentStatus {
                    id: ClientId::parse("11111111-1111-1111-1111-111111111111").expect("uuid"),
                    preset: PresetId::Hiddify,
                    enabled: true,
                    status: hiddify,
                }],
                mihomo: ComponentStatus::new(
                    if running {
                        ComponentPhase::Running
                    } else {
                        ComponentPhase::Stopped
                    },
                    None,
                ),
                tun: ComponentStatus::new(
                    if tun {
                        ComponentPhase::Running
                    } else {
                        ComponentPhase::Stopped
                    },
                    Some("test-tun".into()),
                ),
                dns: ComponentStatus::new(
                    if running {
                        ComponentPhase::Running
                    } else {
                        ComponentPhase::Stopped
                    },
                    None,
                ),
                providers: if running {
                    ProviderSummary {
                        ready: 1,
                        total: 1,
                        rules_loaded: 100,
                        last_refresh: Some(Utc::now()),
                    }
                } else {
                    ProviderSummary::default()
                },
            }
        }

        async fn helper_status(&self) -> Result<HelperStatus, CoreError> {
            if self.helper_missing.load(Ordering::SeqCst) {
                return Ok(HelperStatus::default());
            }
            Ok(HelperStatus {
                available: true,
                authorized: true,
                version: Some("test".into()),
            })
        }

        async fn ensure_hiddify(&self, cancel: CancellationToken) -> Result<(), CoreError> {
            if self.hiddify_missing.load(Ordering::SeqCst) {
                return Err(CoreError::HiddifyNotFound);
            }
            if self.slow_hiddify.load(Ordering::SeqCst) {
                tokio::select! {
                    () = cancel.cancelled() => return Err(CoreError::Cancelled),
                    () = tokio::time::sleep(Duration::from_secs(2)) => {}
                }
            }
            Ok(())
        }

        async fn prepare_runtime(&self) -> Result<RuntimeGeneration, CoreError> {
            Ok(RuntimeGeneration {
                generation_id: Uuid::new_v4(),
                config_sha256: "a".repeat(64),
            })
        }

        async fn validate_runtime(&self, _generation: &RuntimeGeneration) -> Result<(), CoreError> {
            Ok(())
        }

        async fn start_core(&self, _generation: &RuntimeGeneration) -> Result<(), CoreError> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            self.process.store(true, Ordering::SeqCst);
            self.tun.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn stop_core(&self) -> Result<(), CoreError> {
            self.process.store(false, Ordering::SeqCst);
            Ok(())
        }

        async fn stop_user_proxy(&self) -> Result<(), CoreError> {
            self.proxy_stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn clear_hiddify_system_proxy(&self) -> Result<bool, CoreError> {
            self.proxy_clears.fetch_add(1, Ordering::SeqCst);
            Ok(false)
        }

        async fn restore_hiddify_system_proxy(&self) -> Result<(), CoreError> {
            self.proxy_restores.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn core_process(&self) -> Result<ProcessStatus, CoreError> {
            Ok(ProcessStatus {
                running: self.process.load(Ordering::SeqCst),
                pid: Some(42),
            })
        }

        async fn tun_status(&self) -> Result<TunStatus, CoreError> {
            Ok(TunStatus {
                active: self.tun.load(Ordering::SeqCst),
                name: Some("test-tun".into()),
            })
        }

        async fn check_readiness(
            &self,
            _cancel: CancellationToken,
        ) -> Result<ReadinessReport, CoreError> {
            let fail = self.fail_readiness.load(Ordering::SeqCst);
            Ok(ReadinessReport {
                controller_ready: !fail,
                egress_ready: !fail,
                providers: ProviderSummary {
                    ready: u32::from(!fail),
                    total: 1,
                    rules_loaded: 100,
                    last_refresh: Some(Utc::now()),
                },
                exit_ip: Some("203.0.113.10".into()),
            })
        }

        async fn cleanup_owned_state(&self) -> Result<CleanupReport, CoreError> {
            self.cleanups.fetch_add(1, Ordering::SeqCst);
            self.process.store(false, Ordering::SeqCst);
            self.tun.store(false, Ordering::SeqCst);
            Ok(CleanupReport {
                process_stopped: true,
                tun_removed: true,
                dns_restored: true,
                routes_removed: 2,
                warnings: vec![],
            })
        }
    }

    #[tokio::test]
    async fn pause_keeps_hiddify_and_removes_owned_state() {
        let backend = Arc::new(FakeBackend::default());
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        engine.start_stack().await.expect("start accepted");
        engine
            .wait_for_phase(StackPhase::Running, Duration::from_secs(2))
            .await
            .expect("running");

        engine.pause_stack().await.expect("pause accepted");
        engine
            .wait_for_phase(StackPhase::Paused, Duration::from_secs(2))
            .await
            .expect("paused");

        assert!(!backend.tun.load(Ordering::SeqCst));
        assert_eq!(
            engine.snapshot().clients[0].status.phase,
            ComponentPhase::Running
        );
        assert_eq!(backend.cleanups.load(Ordering::SeqCst), 1);
        assert_eq!(backend.proxy_stops.load(Ordering::SeqCst), 0);
        // one clear when start completed, one during pause
        assert_eq!(backend.proxy_clears.load(Ordering::SeqCst), 2);
        assert!(
            engine
                .pause_stack()
                .await
                .expect("idempotent")
                .already_complete
        );
    }

    #[tokio::test]
    async fn resume_from_paused_restores_running() {
        let backend = Arc::new(FakeBackend::default());
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        engine.start_stack().await.expect("start accepted");
        engine
            .wait_for_phase(StackPhase::Running, Duration::from_secs(2))
            .await
            .expect("running");
        engine.pause_stack().await.expect("pause accepted");
        engine
            .wait_for_phase(StackPhase::Paused, Duration::from_secs(2))
            .await
            .expect("paused");

        engine.resume_stack().await.expect("resume accepted");
        engine
            .wait_for_phase(StackPhase::Running, Duration::from_secs(2))
            .await
            .expect("running again");
        assert!(backend.tun.load(Ordering::SeqCst));
        // resume must never restore a Hiddify system proxy: it would bypass
        // the TUN split routing and break DIRECT domains
        assert_eq!(backend.proxy_restores.load(Ordering::SeqCst), 0);
        // start, pause, and the resume's start steps each clear once
        assert_eq!(backend.proxy_clears.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn failed_resume_rolls_back_to_paused() {
        let backend = Arc::new(FakeBackend::default());
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        engine.start_stack().await.expect("start accepted");
        engine
            .wait_for_phase(StackPhase::Running, Duration::from_secs(2))
            .await
            .expect("running");
        engine.pause_stack().await.expect("pause accepted");
        engine
            .wait_for_phase(StackPhase::Paused, Duration::from_secs(2))
            .await
            .expect("paused");

        backend.fail_readiness.store(true, Ordering::SeqCst);
        engine.resume_stack().await.expect("resume accepted");
        let mut receiver = engine.subscribe();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = receiver.borrow().clone();
                if snapshot.phase == StackPhase::Paused && snapshot.last_error.is_some() {
                    return;
                }
                receiver.changed().await.expect("snapshot update");
            }
        })
        .await
        .expect("paused with error");
        assert_eq!(engine.snapshot().phase, StackPhase::Paused);
        assert_eq!(
            engine.snapshot().clients[0].status.phase,
            ComponentPhase::Running
        );
        assert!(!backend.tun.load(Ordering::SeqCst));
        assert_eq!(backend.proxy_stops.load(Ordering::SeqCst), 0);
        assert_eq!(backend.proxy_restores.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn start_and_stop_are_idempotent() {
        let backend = Arc::new(FakeBackend::default());
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        let first = engine.start_stack().await.expect("start accepted");
        let duplicate = engine.start_stack().await.expect("duplicate accepted");
        assert_eq!(first.operation_id, duplicate.operation_id);
        engine
            .wait_for_phase(StackPhase::Running, Duration::from_secs(2))
            .await
            .expect("running");
        assert_eq!(backend.starts.load(Ordering::SeqCst), 1);
        // a completed start clears any Hiddify system proxy exactly once
        assert_eq!(backend.proxy_clears.load(Ordering::SeqCst), 1);
        assert_eq!(backend.proxy_restores.load(Ordering::SeqCst), 0);
        assert!(
            engine
                .start_stack()
                .await
                .expect("idempotent")
                .already_complete
        );

        engine.stop_stack().await.expect("stop accepted");
        engine
            .wait_for_phase(StackPhase::Stopped, Duration::from_secs(2))
            .await
            .expect("stopped");
        assert!(!backend.tun.load(Ordering::SeqCst));
        // one clear when start completed, one during stop
        assert_eq!(backend.proxy_clears.load(Ordering::SeqCst), 2);
        assert_eq!(backend.proxy_stops.load(Ordering::SeqCst), 1);
        assert!(
            engine
                .stop_stack()
                .await
                .expect("idempotent")
                .already_complete
        );
    }

    #[tokio::test]
    async fn failed_readiness_rolls_back_owned_state() {
        let backend = Arc::new(FakeBackend::default());
        backend.fail_readiness.store(true, Ordering::SeqCst);
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        engine.start_stack().await.expect("start accepted");
        let mut receiver = engine.subscribe();
        tokio::time::timeout(Duration::from_secs(2), async {
            while receiver.borrow().phase != StackPhase::Error {
                receiver.changed().await.expect("snapshot update");
            }
        })
        .await
        .expect("error phase");
        assert!(!backend.tun.load(Ordering::SeqCst));
        assert_eq!(backend.cleanups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn conflicting_operations_are_rejected_while_start_is_busy() {
        let backend = Arc::new(FakeBackend::default());
        backend.slow_hiddify.store(true, Ordering::SeqCst);
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        let first = engine.start_stack().await.expect("start accepted");
        let duplicate = engine.start_stack().await.expect("duplicate start");
        assert_eq!(first.operation_id, duplicate.operation_id);
        assert_eq!(engine.snapshot().busy, Some(LifecycleBusy::Connecting));
        assert!(matches!(
            engine.stop_stack().await,
            Err(CoreError::OperationInProgress)
        ));
        assert!(matches!(
            engine.pause_stack().await,
            Err(CoreError::OperationInProgress)
        ));
        engine
            .wait_for_phase(StackPhase::Running, Duration::from_secs(3))
            .await
            .expect("running");
        assert_eq!(engine.snapshot().busy, None);
        assert_eq!(backend.starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn operation_timeout_clears_the_lifecycle_lock() {
        let backend = Arc::new(FakeBackend::default());
        backend.slow_hiddify.store(true, Ordering::SeqCst);
        let engine = Engine::new_with_timeouts(
            Arc::clone(&backend),
            &tokio::runtime::Handle::current(),
            Duration::from_millis(40),
            Duration::from_secs(1),
        );
        engine.start_stack().await.expect("start accepted");
        let mut receiver = engine.subscribe();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = receiver.borrow().clone();
                if snapshot.phase == StackPhase::Error && snapshot.busy.is_none() {
                    return;
                }
                receiver.changed().await.expect("snapshot update");
            }
        })
        .await
        .expect("timeout recovered");
        let error = engine.snapshot().last_error.expect("timeout error");
        assert_eq!(error.code, ErrorCode::OperationTimeout);
        engine
            .stop_stack()
            .await
            .expect("controls recover after timeout");
    }

    #[tokio::test]
    async fn reserve_blocks_tray_like_conflicting_entry_points() {
        let backend = Arc::new(FakeBackend::default());
        backend.slow_hiddify.store(true, Ordering::SeqCst);
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        engine
            .reserve_lifecycle(LifecycleBusy::Connecting)
            .await
            .expect("reserved");
        assert!(matches!(
            engine.reserve_lifecycle(LifecycleBusy::Disconnecting).await,
            Err(CoreError::OperationInProgress)
        ));
        assert!(matches!(
            engine.pause_stack().await,
            Err(CoreError::OperationInProgress)
        ));
        engine.release_lifecycle(LifecycleBusy::Connecting).await;
        assert_eq!(engine.snapshot().busy, None);
    }

    #[tokio::test]
    async fn startup_reconciliation_removes_orphans() {
        let backend = Arc::new(FakeBackend::default());
        backend.process.store(true, Ordering::SeqCst);
        backend.tun.store(true, Ordering::SeqCst);
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        engine
            .reconcile_startup()
            .await
            .expect("reconcile accepted");
        engine
            .wait_for_phase(StackPhase::Stopped, Duration::from_secs(2))
            .await
            .expect("stopped");
        assert_eq!(backend.cleanups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn startup_without_helper_reports_exact_component_states() {
        let backend = Arc::new(FakeBackend::default());
        backend.helper_missing.store(true, Ordering::SeqCst);
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());

        engine
            .reconcile_startup()
            .await
            .expect("reconcile accepted");
        let snapshot = engine
            .wait_for_phase(StackPhase::Stopped, Duration::from_secs(2))
            .await
            .expect("stopped");

        assert_eq!(snapshot.helper.phase, ComponentPhase::Unavailable);
        assert_eq!(snapshot.clients[0].status.phase, ComponentPhase::Running);
        assert_eq!(snapshot.mihomo.phase, ComponentPhase::Stopped);
        assert_eq!(snapshot.tun.phase, ComponentPhase::Stopped);
        assert_eq!(snapshot.dns.phase, ComponentPhase::Stopped);
        assert!(snapshot.last_error.is_none());
    }

    #[test]
    fn snapshot_contract_serializes_in_snake_case() {
        let snapshot = StackSnapshot {
            phase: StackPhase::CheckingReadiness,
            ..StackSnapshot::default()
        };
        let value = serde_json::to_value(snapshot).expect("serialize");
        assert_eq!(value["phase"], "checking_readiness");
        assert_eq!(value["busy"], serde_json::Value::Null);
        assert_eq!(value["operation_stage"], serde_json::Value::Null);
    }

    async fn collect_stages_until(
        receiver: &mut watch::Receiver<StackSnapshot>,
        desired: StackPhase,
    ) -> Vec<OperationStage> {
        let mut stages = Vec::new();
        loop {
            receiver.changed().await.expect("snapshot update");
            let snapshot = receiver.borrow().clone();
            if let Some(stage) = snapshot.operation_stage {
                if stages.last() != Some(&stage) {
                    stages.push(stage);
                }
            }
            if snapshot.phase == desired && snapshot.busy.is_none() {
                return stages;
            }
        }
    }

    #[tokio::test]
    async fn start_and_stop_publish_real_operation_stages() {
        let backend = Arc::new(FakeBackend::default());
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        let mut receiver = engine.subscribe();
        let collect = collect_stages_until(&mut receiver, StackPhase::Running);
        let start = engine.start_stack();
        let (stages, accepted) = tokio::join!(collect, start);
        accepted.expect("start accepted");
        assert!(stages.contains(&OperationStage::StartingClient));
        assert!(stages.contains(&OperationStage::StartingCore));
        assert!(stages.contains(&OperationStage::CheckingReadiness));

        let collect = collect_stages_until(&mut receiver, StackPhase::Stopped);
        let stop = engine.stop_stack();
        let (stages, accepted) = tokio::join!(collect, stop);
        accepted.expect("stop accepted");
        assert!(stages.contains(&OperationStage::StoppingCore));
        assert!(stages.contains(&OperationStage::StoppingProxy));
        assert!(stages.contains(&OperationStage::CleaningUp));
        assert_eq!(engine.snapshot().operation_stage, None);
    }

    #[test]
    fn engine_can_be_constructed_outside_an_entered_runtime() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let engine = Engine::new(Arc::new(FakeBackend::default()), runtime.handle());

        assert_eq!(engine.snapshot().phase, StackPhase::Uninitialized);
    }

    #[tokio::test]
    async fn apply_user_rules_is_a_no_op_until_the_stack_is_live() {
        let backend = Arc::new(FakeBackend::default());
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        engine.apply_user_rules().await.expect("stopped");
        assert_eq!(backend.starts.load(Ordering::SeqCst), 0);
        engine.start_stack().await.expect("start");
        engine
            .wait_for_phase(StackPhase::Running, Duration::from_secs(2))
            .await
            .expect("running");
        let starts = backend.starts.load(Ordering::SeqCst);
        engine.apply_user_rules().await.expect("live apply");
        assert_eq!(backend.starts.load(Ordering::SeqCst), starts + 1);
    }

    #[test]
    fn missing_hiddify_and_mihomo_ask_the_ui_to_install() {
        let hiddify = CoreError::HiddifyNotFound.to_app_error(Uuid::nil());
        assert_eq!(hiddify.code, ErrorCode::HiddifyNotFound);
        assert_eq!(hiddify.remediation, Some(Remediation::InstallDependency));
        let mihomo = CoreError::MihomoNotFound.to_app_error(Uuid::nil());
        assert_eq!(mihomo.code, ErrorCode::MihomoNotFound);
        assert_eq!(mihomo.remediation, Some(Remediation::InstallDependency));
    }

    #[test]
    fn readiness_postcondition_names_process_versus_tun() {
        assert_eq!(
            core_tun_missing_message(
                &ProcessStatus {
                    running: false,
                    pid: None
                },
                &TunStatus {
                    active: false,
                    name: Some("clash-iran".into())
                }
            ),
            "Mihomo process disappeared during readiness checks"
        );
        assert_eq!(
            core_tun_missing_message(
                &ProcessStatus {
                    running: true,
                    pid: Some(7)
                },
                &TunStatus {
                    active: false,
                    name: Some("clash-iran".into())
                }
            ),
            "TUN clash-iran was not active after the controller was ready"
        );
    }

    #[test]
    fn controller_timeout_is_not_an_internal_error() {
        let timeout = CoreError::ControllerTimeout.to_app_error(Uuid::nil());
        assert_eq!(timeout.code, ErrorCode::ControllerTimeout);
        assert_eq!(timeout.message_key, "errors.controllerTimeout");
        let wrapped =
            CoreError::Platform("Mihomo readiness check timed out: controller unavailable".into())
                .to_app_error(Uuid::nil());
        assert_eq!(wrapped.message_key, "errors.internal");
    }

    #[tokio::test]
    async fn missing_hiddify_marks_the_stack_error() {
        let backend = Arc::new(FakeBackend::default());
        backend.hiddify_missing.store(true, Ordering::SeqCst);
        let engine = Engine::new(Arc::clone(&backend), &tokio::runtime::Handle::current());
        engine.start_stack().await.expect("start accepted");
        let mut receiver = engine.subscribe();
        tokio::time::timeout(Duration::from_secs(2), async {
            while receiver.borrow().phase != StackPhase::Error {
                receiver.changed().await.expect("snapshot update");
            }
        })
        .await
        .expect("error phase");
        let error = engine.snapshot().last_error.expect("last error");
        assert_eq!(error.code, ErrorCode::HiddifyNotFound);
        assert_eq!(error.remediation, Some(Remediation::InstallDependency));
    }
}
