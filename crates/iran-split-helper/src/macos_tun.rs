//! Persist the kernel-assigned device from this core's startup output. Never
//! enumerate or delete unrelated `utun` devices by name prefix.
use super::HelperServiceError;
use std::{fs, io::Write, path::Path};

const FILE: &str = "owned-macos-tun";

pub(crate) async fn remember_started_device(
    runtime: &Path,
    logs: &tokio::sync::Mutex<std::collections::VecDeque<iran_split_ipc::ServiceLogEntry>>,
    launch_id: uuid::Uuid,
    budget: std::time::Duration,
) -> Result<(), HelperServiceError> {
    let deadline = std::time::Instant::now() + budget;
    let launch = launch_id.to_string();
    loop {
        let device = logs
            .lock()
            .await
            .iter()
            .rev()
            .filter(|entry| entry.fields.get("launch_id") == Some(&launch))
            .find_map(|entry| {
                entry
                    .fields
                    .get("message")
                    .and_then(|message| device_from_message(message))
                    .map(str::to_owned)
            });
        if let Some(device) = device {
            remember(runtime, &device)?;
            tracing::info!(event = "helper.native_tun_recorded", section = "helper_network",
                initiator = "helper_process", cause = "owned_core_startup_output", trace_id = %launch_id,
                trace_route = "helper_process->start->native_tun_identity", "recorded the owned kernel tunnel identity");
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(HelperServiceError::Process(
                "macOS core did not report its native tunnel identity".into(),
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

pub(crate) fn device_from_message(message: &str) -> Option<&str> {
    let suffix = message.split_once("Tun adapter listening at: ")?.1;
    let end = suffix
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(suffix.len());
    let name = &suffix[..end];
    valid_device(name).then_some(name)
}

fn valid_device(name: &str) -> bool {
    name.strip_prefix("utun").is_some_and(|index| {
        !index.is_empty() && index.len() <= 5 && index.bytes().all(|c| c.is_ascii_digit())
    })
}

pub(crate) fn remember(runtime: &Path, name: &str) -> Result<(), HelperServiceError> {
    if !valid_device(name) {
        return Err(HelperServiceError::Process(
            "invalid native macOS tunnel identity".into(),
        ));
    }
    fs::create_dir_all(runtime)?;
    let mut staged = tempfile::NamedTempFile::new_in(runtime)?;
    staged.write_all(name.as_bytes())?;
    staged.as_file().sync_all()?;
    staged
        .persist(runtime.join(FILE))
        .map_err(|error| error.error)?;
    Ok(())
}

pub(crate) fn saved(runtime: &Path) -> Result<Option<String>, HelperServiceError> {
    match fs::read_to_string(runtime.join(FILE)) {
        Ok(name) if valid_device(&name) => Ok(Some(name)),
        Ok(_) => Err(HelperServiceError::Process(
            "invalid saved macOS tunnel identity".into(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn retire(runtime: &Path) -> Result<(), HelperServiceError> {
    match fs::remove_file(runtime.join(FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn repeated_generation_cannot_reuse_a_previous_process_device() {
        use iran_split_ipc::ServiceLogEntry;
        use std::{
            collections::{BTreeMap, VecDeque},
            time::Duration,
        };
        let runtime = tempfile::tempdir().unwrap();
        let previous = uuid::Uuid::new_v4();
        let launch = uuid::Uuid::new_v4();
        let entry = |id: uuid::Uuid, device| ServiceLogEntry {
            timestamp: "fixture".into(),
            level: "info".into(),
            event: "mihomo_output".into(),
            fields: BTreeMap::from([
                ("launch_id".into(), id.to_string()),
                (
                    "message".into(),
                    format!("[TUN] Tun adapter listening at: {device}([fixture])"),
                ),
            ]),
        };
        let logs = tokio::sync::Mutex::new(VecDeque::from([entry(previous, "utun7")]));
        assert!(
            remember_started_device(runtime.path(), &logs, launch, Duration::ZERO)
                .await
                .is_err()
        );
        assert_eq!(saved(runtime.path()).unwrap(), None);
        logs.lock().await.push_back(entry(launch, "utun9"));
        remember_started_device(runtime.path(), &logs, launch, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(saved(runtime.path()).unwrap().as_deref(), Some("utun9"));
    }

    #[test]
    fn only_a_reported_kernel_device_can_be_persisted_and_retired() {
        let runtime = tempfile::tempdir().unwrap();
        assert_eq!(saved(runtime.path()).unwrap(), None);
        for message in [
            "tun enabled",
            "Tun adapter listening at: en0",
            "Tun adapter listening at: utun",
            "Tun adapter listening at: ../utun9",
        ] {
            assert_eq!(device_from_message(message), None);
        }
        let name = device_from_message("time=fixture level=info msg=\"[TUN] Tun adapter listening at: utun9([fixture], mtu: 9000)\"").unwrap();
        remember(runtime.path(), name).unwrap();
        assert_eq!(saved(runtime.path()).unwrap().as_deref(), Some("utun9"));
        assert!(remember(runtime.path(), "en0").is_err());
        assert_eq!(saved(runtime.path()).unwrap().as_deref(), Some("utun9"));
        retire(runtime.path()).unwrap();
        retire(runtime.path()).unwrap();
        assert_eq!(saved(runtime.path()).unwrap(), None);
    }
}
