use chrono::Utc;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::{
    fmt::Display,
    fs::{self, File, OpenOptions},
    future::Future,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Instant,
};
use tracing::{error, info, info_span, Instrument, Subscriber};
use tracing_subscriber::{fmt::MakeWriter, EnvFilter};
use uuid::Uuid;

const DEFAULT_FILTER: &str = "info,hyper=warn,reqwest=warn,rustls=warn,tao=warn,wry=warn";

static ACTIVE_SESSION: OnceLock<ActiveSession> = OnceLock::new();
static SESSION_CLOSED: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
struct ActiveSession {
    id: Uuid,
    log: DebugLog,
}

#[derive(Clone, Debug)]
struct DebugLog {
    inner: Arc<Mutex<LogState>>,
}

#[derive(Debug)]
struct LogState {
    file: Option<File>,
    path: PathBuf,
    bytes_written: u64,
}

impl DebugLog {
    fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = open_append_log(path)?;
        let bytes_written = file.metadata()?.len();
        Ok(Self {
            inner: Arc::new(Mutex::new(LogState {
                file: Some(file),
                path: path.to_owned(),
                bytes_written,
            })),
        })
    }

    fn append(&self, bytes: &[u8]) -> io::Result<()> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("debug log lock is poisoned"))?;
        let incoming = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        {
            let file = log_file(&mut state)?;
            file.write_all(bytes)?;
            file.flush()?;
        }
        state.bytes_written = state.bytes_written.saturating_add(incoming);
        Ok(())
    }

    fn flush(&self) -> io::Result<()> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("debug log lock is poisoned"))?;
        log_file(&mut state)?.flush()
    }

    fn copy_to(&self, destination: &Path) -> io::Result<u64> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("debug log lock is poisoned"))?;
        log_file(&mut state)?.flush()?;
        fs::copy(&state.path, destination)
    }

    fn status(&self) -> io::Result<DebugLogStatus> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("debug log lock is poisoned"))?;
        log_file(&mut state)?.flush()?;
        state.bytes_written = log_file(&mut state)?.metadata()?.len();
        Ok(DebugLogStatus {
            path: state.path.to_string_lossy().into_owned(),
            size_bytes: state.bytes_written,
        })
    }

    fn clear(&self) -> io::Result<()> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("debug log lock is poisoned"))?;
        if let Some(mut file) = state.file.take() {
            file.flush()?;
        }
        let path = state.path.clone();
        // Windows cannot SetEndOfFile on a handle opened with FILE_APPEND_DATA.
        // Close the append handle, truncate, then reopen in append mode so the
        // Diagnostics delete action keeps logging immediately.
        drop(OpenOptions::new().write(true).truncate(true).open(&path)?);
        state.file = Some(open_append_log(&path)?);
        state.bytes_written = 0;
        Ok(())
    }
}

#[derive(Debug)]
struct EventWriter {
    log: DebugLog,
    bytes: Vec<u8>,
    committed: bool,
}

impl EventWriter {
    fn commit(&mut self) -> io::Result<()> {
        if self.committed {
            return Ok(());
        }
        self.committed = true;
        self.log.append(&sanitize_formatted_events(&self.bytes))
    }
}

impl Write for EventWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.commit()
    }
}

impl Drop for EventWriter {
    fn drop(&mut self) {
        if let Err(error) = self.commit() {
            eprintln!("BiFlow could not write debug.log: {error}");
        }
    }
}

impl<'writer> MakeWriter<'writer> for DebugLog {
    type Writer = EventWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        EventWriter {
            log: self.clone(),
            bytes: Vec::with_capacity(512),
            committed: false,
        }
    }
}

fn sanitize_formatted_events(bytes: &[u8]) -> Vec<u8> {
    let mut sanitized = Vec::with_capacity(bytes.len());
    for raw_line in bytes.split(|byte| *byte == b'\n') {
        if raw_line.is_empty() {
            continue;
        }
        let mut event = match serde_json::from_slice::<Value>(raw_line) {
            Ok(event) => event,
            Err(error) => serde_json::json!({
                "timestamp": Utc::now().to_rfc3339(),
                "level": "ERROR",
                "message": "tracing emitted a malformed JSON event",
                "event": "log.format_failed",
                "section": "diagnostics",
                "initiator": "event_writer",
                "cause": error.to_string(),
                "trace_route": "tracing_subscriber->event_writer",
            }),
        };
        normalize_event(&mut event);
        redact_value(None, &mut event);
        if let Ok(mut encoded) = serde_json::to_vec(&event) {
            sanitized.append(&mut encoded);
            sanitized.push(b'\n');
        }
    }
    sanitized
}

fn normalize_event(event: &mut Value) {
    let span_field = |name: &str| {
        event
            .get("span")
            .and_then(|span| span.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                event
                    .get("spans")
                    .and_then(Value::as_array)
                    .and_then(|spans| spans.last())
                    .and_then(|span| span.get(name))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
    };
    let target = event
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or("rust_runtime")
        .to_owned();
    let level = event
        .get("level")
        .and_then(Value::as_str)
        .unwrap_or("INFO")
        .to_owned();
    let section = span_field("section").unwrap_or_else(|| target.clone());
    let initiator = span_field("initiator").unwrap_or_else(|| target.clone());
    let trace_id = span_field("trace_id").unwrap_or_else(|| active_session_id().to_string());
    let trace_route = span_field("trace_route").unwrap_or_else(|| format!("{target}->debug.log"));
    let cause = if level == "WARN" || level == "ERROR" {
        "unspecified"
    } else {
        "none"
    };

    if let Some(fields) = event.as_object_mut() {
        fields
            .entry("event")
            .or_insert_with(|| Value::String("rust.event".into()));
        fields
            .entry("section")
            .or_insert_with(|| Value::String(section));
        fields
            .entry("initiator")
            .or_insert_with(|| Value::String(initiator));
        fields
            .entry("cause")
            .or_insert_with(|| Value::String(cause.into()));
        fields
            .entry("trace_id")
            .or_insert_with(|| Value::String(trace_id));
        fields
            .entry("trace_route")
            .or_insert_with(|| Value::String(trace_route));
    }
}

fn redact_value(key: Option<&str>, value: &mut Value) {
    if key.is_some_and(sensitive_key) {
        *value = Value::String("<redacted>".into());
        return;
    }
    match value {
        Value::String(text) => *text = redact_text(text),
        Value::Array(values) => {
            for value in values {
                redact_value(None, value);
            }
        }
        Value::Object(fields) => {
            for (field, value) in fields {
                redact_value(Some(field), value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "cookie",
        "credential",
        "password",
        "secret",
        "subscription",
        "token",
    ]
    .iter()
    .any(|sensitive| key.contains(sensitive))
}

fn redact_text(text: &str) -> String {
    static SECRET_ASSIGNMENT: OnceLock<Regex> = OnceLock::new();
    static URL: OnceLock<Regex> = OnceLock::new();
    let assignment = SECRET_ASSIGNMENT.get_or_init(|| {
        Regex::new(
            r#"(?i)(authorization|cookie|credential|password|secret|subscription|token)\s*[:=]\s*(\"[^\"]*\"|'[^']*'|[^\s,;]+)"#,
        )
        .expect("secret redaction regex is valid")
    });
    let url = URL.get_or_init(|| {
        Regex::new(r#"(?i)https?://[^\s\"']+"#).expect("URL redaction regex is valid")
    });
    let redacted = assignment.replace_all(text, "$1=<redacted>");
    url.replace_all(&redacted, "<redacted-url>").into_owned()
}

fn subscriber(log: DebugLog) -> impl Subscriber + Send + Sync {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(DEFAULT_FILTER))
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(true)
        .with_ansi(false)
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_writer(log)
        .finish()
}

pub fn default_log_path() -> Result<PathBuf, String> {
    // A dev run logs inside its own profile; it must never write into (or
    // reveal) the installed app's data directory.
    if let Some(profile) = std::env::var_os("BIFLOW_DEV_PROFILE").filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(profile).join("data").join("debug.log"));
    }
    dirs::data_local_dir()
        .map(|path| path.join("biflow").join("debug.log"))
        .ok_or_else(|| "local data directory is unavailable for debug.log".into())
}

pub fn initialize(path: &Path, app_version: &str) -> Result<(), String> {
    let log = DebugLog::open(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    let previous_bytes = log
        .status()
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?
        .size_bytes;
    tracing::subscriber::set_global_default(subscriber(log.clone()))
        .map_err(|error| format!("cannot install diagnostic subscriber: {error}"))?;

    let session_id = Uuid::new_v4();
    ACTIVE_SESSION
        .set(ActiveSession {
            id: session_id,
            log,
        })
        .map_err(|_| "diagnostic session was initialized more than once".to_owned())?;
    install_panic_hook();
    info!(
        target: "biflow::diagnostics",
        event = "session.opened",
        section = "lifecycle",
        initiator = "application_process",
        cause = "process_start",
        trace_id = %session_id,
        trace_route = "application_process->rust_runtime->tauri_setup",
        app_version,
        platform = std::env::consts::OS,
        architecture = std::env::consts::ARCH,
        process_id = std::process::id(),
        log_path = %path.display(),
        previous_bytes,
        "diagnostic session opened"
    );
    Ok(())
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let cause = panic
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| panic.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        let location = panic
            .location()
            .map_or_else(|| "unknown".to_owned(), ToString::to_string);
        error!(
            target: "biflow::diagnostics",
            event = "runtime.panic",
            section = "runtime",
            initiator = "rust_panic_hook",
            cause,
            trace_id = %active_session_id(),
            trace_route = "rust_runtime->panic_hook",
            location,
            "Rust panic captured"
        );
        flush();
        previous(panic);
    }));
}

fn active_session_id() -> Uuid {
    ACTIVE_SESSION
        .get()
        .map_or_else(Uuid::nil, |session| session.id)
}

pub async fn trace_action<T, E, F>(
    section: &'static str,
    initiator: &'static str,
    action: &'static str,
    future: F,
) -> Result<T, E>
where
    E: Display,
    F: Future<Output = Result<T, E>>,
{
    let trace_id = Uuid::new_v4();
    let trace_route = format!("{initiator}->{section}->{action}");
    let span = info_span!(
        target: "biflow::action",
        "event_trace",
        session_id = %active_session_id(),
        trace_id = %trace_id,
        section,
        initiator,
        action,
        trace_route,
    );
    async move {
        let started = Instant::now();
        info!(
            event = "action.started",
            cause = "requested",
            "action started"
        );
        match future.await {
            Ok(value) => {
                info!(
                    event = "action.completed",
                    cause = "none",
                    elapsed_ms = elapsed_millis(started),
                    "action completed"
                );
                Ok(value)
            }
            Err(action_error) => {
                error!(
                    event = "action.failed",
                    cause = %action_error,
                    elapsed_ms = elapsed_millis(started),
                    "action failed"
                );
                Err(action_error)
            }
        }
    }
    .instrument(span)
    .await
}

pub fn trace_sync<T, E>(
    section: &'static str,
    initiator: &'static str,
    action: &'static str,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, E>
where
    E: Display,
{
    let trace_id = Uuid::new_v4();
    let trace_route = format!("{initiator}->{section}->{action}");
    let span = info_span!(
        target: "biflow::action",
        "event_trace",
        session_id = %active_session_id(),
        trace_id = %trace_id,
        section,
        initiator,
        action,
        trace_route,
    );
    let _entered = span.enter();
    let started = Instant::now();
    info!(
        event = "action.started",
        cause = "requested",
        "action started"
    );
    match operation() {
        Ok(value) => {
            info!(
                event = "action.completed",
                cause = "none",
                elapsed_ms = elapsed_millis(started),
                "action completed"
            );
            Ok(value)
        }
        Err(action_error) => {
            error!(
                event = "action.failed",
                cause = %action_error,
                elapsed_ms = elapsed_millis(started),
                "action failed"
            );
            Err(action_error)
        }
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub fn exit_requested(code: Option<i32>) {
    info!(
        target: "biflow::diagnostics",
        event = "session.exit_requested",
        section = "lifecycle",
        initiator = "tauri_event_loop",
        cause = "application_exit",
        trace_id = %active_session_id(),
        trace_route = "tauri_event_loop->application_exit",
        exit_code = ?code,
        "application exit requested"
    );
    flush();
}

pub fn close_session() {
    if SESSION_CLOSED.swap(true, Ordering::SeqCst) {
        return;
    }
    info!(
        target: "biflow::diagnostics",
        event = "session.closed",
        section = "lifecycle",
        initiator = "tauri_event_loop",
        cause = "event_loop_exit",
        trace_id = %active_session_id(),
        trace_route = "tauri_event_loop->diagnostics->debug.log",
        "diagnostic session closed"
    );
    flush();
}

pub fn flush() {
    if let Some(session) = ACTIVE_SESSION.get() {
        if let Err(error) = session.log.flush() {
            eprintln!("BiFlow could not flush debug.log: {error}");
        }
    }
}

pub fn copy_log(destination: &Path) -> Result<u64, String> {
    ACTIVE_SESSION
        .get()
        .ok_or_else(|| "diagnostic session is not initialized".to_owned())?
        .log
        .copy_to(destination)
        .map_err(|error| format!("cannot copy debug.log: {error}"))
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugLogStatus {
    pub path: String,
    pub size_bytes: u64,
}

pub fn status() -> Result<DebugLogStatus, String> {
    ACTIVE_SESSION
        .get()
        .ok_or_else(|| "diagnostic session is not initialized".to_owned())?
        .log
        .status()
        .map_err(|error| format!("cannot inspect debug.log: {error}"))
}

pub fn clear() -> Result<DebugLogStatus, String> {
    let session = ACTIVE_SESSION
        .get()
        .ok_or_else(|| "diagnostic session is not initialized".to_owned())?;
    session
        .log
        .clear()
        .map_err(|error| format!("cannot clear debug.log: {error}"))?;
    info!(
        target: "biflow::diagnostics",
        event = "log.cleared",
        section = "diagnostics",
        initiator = "user",
        cause = "delete_log_requested",
        trace_id = %Uuid::new_v4(),
        trace_route = "diagnostics_ui->delete_debug_log->debug.log",
        "previous debug log contents deleted by the user"
    );
    status()
}

pub fn reveal() -> Result<DebugLogStatus, String> {
    let status = status()?;
    let path = Path::new(&status.path);
    spawn_reveal(path)?;
    Ok(status)
}

fn open_append_log(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
}

fn log_file(state: &mut LogState) -> io::Result<&mut File> {
    state
        .file
        .as_mut()
        .ok_or_else(|| io::Error::other("debug log is closed"))
}

fn spawn_reveal(path: &Path) -> Result<(), String> {
    let (program, arguments) = reveal_command(path);
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        for argument in arguments {
            command.raw_arg(argument);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        command.args(arguments);
    }
    command
        .spawn()
        .map_err(|error| format!("cannot show debug.log: {error}"))?;
    Ok(())
}

fn reveal_command(path: &Path) -> (&'static str, Vec<String>) {
    #[cfg(target_os = "linux")]
    {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        ("xdg-open", vec![parent.to_string_lossy().into_owned()])
    }
    #[cfg(target_os = "windows")]
    {
        ("explorer.exe", vec![windows_reveal_select(path)])
    }
}

/// Explorer treats `/select,C:\dir/file` as a single unknown switch and opens
/// This PC. Normalize separators and keep `/select,` unquoted via `raw_arg`.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_reveal_select(path: &Path) -> String {
    format!("/select,{}", path.to_string_lossy().replace('/', "\\"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn existing_log_is_preserved_and_new_events_are_flushed_json() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("debug.log");
        let previous = serde_json::json!({"event": "previous.session"});
        fs::write(&path, format!("{previous}\n")).expect("previous log");
        let log = DebugLog::open(&path).expect("persistent log");
        let diagnostic_subscriber = subscriber(log.clone());
        tracing::subscriber::with_default(diagnostic_subscriber, || {
            info!(
                event = "test.completed",
                section = "diagnostics_test",
                initiator = "unit_test",
                cause = "none",
                trace_id = %Uuid::nil(),
                trace_route = "unit_test->diagnostics",
                "test event"
            );
        });
        log.flush().expect("flush log");

        let source = fs::read_to_string(path).expect("read log");
        assert!(source.contains("previous.session"));
        let event: Value = serde_json::from_str(source.lines().last().expect("latest event"))
            .expect("valid JSON line");
        assert_eq!(event["event"], "test.completed");
        assert_eq!(event["section"], "diagnostics_test");
        assert_eq!(event["initiator"], "unit_test");
        assert_eq!(event["cause"], "none");
        assert!(event["timestamp"].is_string());
    }

    #[test]
    fn clear_removes_previous_contents_and_keeps_the_file_writable() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("debug.log");
        let log = DebugLog::open(&path).expect("persistent log");
        log.append(b"{\"event\":\"before_clear\"}\n")
            .expect("append old event");
        assert!(log.status().expect("status").size_bytes > 0);
        log.clear().expect("clear log");
        assert_eq!(log.status().expect("cleared status").size_bytes, 0);
        log.append(b"{\"event\":\"after_clear\"}\n")
            .expect("append new event");

        let source = fs::read_to_string(&path).expect("read log");
        assert!(!source.contains("before_clear"));
        assert!(source.contains("after_clear"));
    }

    #[test]
    fn event_writer_redacts_sensitive_fields_urls_and_error_text() {
        let event = b"{\"event\":\"failure\",\"controller_secret\":\"top-secret\",\"cause\":\"token=abc https://example.com/private?id=1\"}\n";
        let sanitized = sanitize_formatted_events(event);
        let source = String::from_utf8(sanitized).expect("UTF-8 JSON");
        assert!(!source.contains("top-secret"));
        assert!(!source.contains("token=abc"));
        assert!(!source.contains("example.com"));
        assert!(source.contains("<redacted>"));
        assert!(source.contains("<redacted-url>"));
    }

    #[test]
    fn legacy_events_receive_the_required_diagnostic_fields() {
        let sanitized = sanitize_formatted_events(
            b"{\"timestamp\":\"2026-01-01T00:00:00Z\",\"level\":\"WARN\",\"target\":\"legacy_module\",\"message\":\"legacy warning\"}\n",
        );
        let event: Value = serde_json::from_slice(&sanitized).expect("normalized event");
        assert_eq!(event["event"], "rust.event");
        assert_eq!(event["section"], "legacy_module");
        assert_eq!(event["initiator"], "legacy_module");
        assert_eq!(event["cause"], "unspecified");
        assert!(event["trace_id"].is_string());
        assert_eq!(event["trace_route"], "legacy_module->debug.log");
    }

    #[test]
    fn reveal_command_opens_the_containing_folder() {
        let path = Path::new("/tmp/biflow/debug.log");
        let (program, arguments) = reveal_command(path);
        #[cfg(target_os = "linux")]
        {
            assert_eq!(program, "xdg-open");
            assert_eq!(arguments, vec!["/tmp/biflow"]);
        }
        #[cfg(target_os = "windows")]
        {
            assert_eq!(program, "explorer.exe");
            assert_eq!(arguments, vec![windows_reveal_select(path)]);
        }
    }

    #[test]
    fn default_log_path_uses_native_separators() {
        let path = default_log_path().expect("local data directory");
        let display = path.to_string_lossy();
        assert!(display.ends_with("debug.log"));
        if display.as_bytes().get(1) == Some(&b':') {
            assert!(
                !display[2..].contains('/'),
                "mixed separators after drive: {display}"
            );
        }
    }

    #[test]
    fn windows_reveal_select_normalizes_mixed_separators() {
        let path = Path::new(r"C:\Users\name\AppData\Local\biflow/debug.log");
        let select = windows_reveal_select(path);
        assert!(select.starts_with("/select,"));
        let selected = select.strip_prefix("/select,").expect("select prefix");
        assert!(
            !selected.get(2..).is_some_and(|rest| rest.contains('/')),
            "Explorer /select path still mixed: {select}"
        );
        assert!(selected.contains('\\'));
        assert!(selected.ends_with("debug.log"));
    }
}
