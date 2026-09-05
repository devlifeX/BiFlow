use async_trait::async_trait;
use clap::{Parser, Subcommand};
use iran_split_config::AppConfig;
use iran_split_core::{
    CleanupReport, ComponentPhase, ComponentStatus, CoreError, Engine, HelperStatus,
    PlatformBackend, ProcessStatus, ProviderSummary, ReadinessReport, RuntimeGeneration,
    RuntimeHealth, StackPhase, TunStatus,
};
use iran_split_rules::{Outbound, RoutePinsDocument, RuleSet};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(version, about = "Internal compatibility CLI for Iran Split Desktop")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Runs a deterministic lifecycle vertical slice without privileged networking.
    Demo,
    /// Validates a configuration TOML file.
    ValidateConfig { path: std::path::PathBuf },
    /// Resolves the intended rule decision using the offline bootstrap snapshot.
    Route { target: String },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Arguments::parse().command {
        Command::Demo => demo().await?,
        Command::ValidateConfig { path } => {
            let source = std::fs::read_to_string(path)?;
            let config: AppConfig = toml::from_str(&source)?;
            let issues = config.validate();
            println!("{}", serde_json::to_string_pretty(&issues)?);
            if !issues.is_empty() {
                std::process::exit(2);
            }
        }
        Command::Route { target } => {
            let domains = include_str!("../../../resources/rules/iran-domains.txt")
                .lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|line| line.trim_start_matches("+.").to_owned());
            let catalog = include_str!("../../../resources/rules/iran-business-domains.txt")
                .lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|line| line.trim_start_matches("+.").to_owned());
            let cidrs = include_str!("../../../resources/rules/private.txt")
                .lines()
                .chain(include_str!("../../../resources/rules/iran-networks.txt").lines())
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::parse)
                .collect::<Result<Vec<_>, _>>()?;
            let rules = RuleSet::from_sources(
                &RoutePinsDocument::default(),
                domains,
                cidrs,
                catalog,
                Outbound::Direct,
                &HashSet::new(),
            );
            println!("{}", serde_json::to_string_pretty(&rules.decide(&target)?)?);
        }
    }
    Ok(())
}

async fn demo() -> Result<(), CoreError> {
    let engine = Engine::new(
        Arc::new(DemoBackend::default()),
        &tokio::runtime::Handle::current(),
    );
    let mut updates = engine.subscribe();
    let printer = tokio::spawn(async move {
        while updates.changed().await.is_ok() {
            let snapshot = updates.borrow().clone();
            println!(
                "{}",
                serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".into())
            );
            if snapshot.phase == StackPhase::Running {
                break;
            }
        }
    });
    engine.start_stack().await?;
    engine
        .wait_for_phase(StackPhase::Running, Duration::from_secs(5))
        .await?;
    printer
        .await
        .map_err(|error| CoreError::Platform(error.to_string()))?;
    engine.stop_stack().await?;
    engine
        .wait_for_phase(StackPhase::Stopped, Duration::from_secs(5))
        .await?;
    Ok(())
}

#[derive(Debug, Default)]
struct DemoBackend {
    running: Mutex<bool>,
}

#[async_trait]
impl PlatformBackend for DemoBackend {
    async fn runtime_health(&self) -> RuntimeHealth {
        let running = *self.running.lock().await;
        RuntimeHealth {
            helper: ComponentStatus::new(ComponentPhase::Running, Some("Helper demo".into())),
            clients: Vec::new(),
            mihomo: ComponentStatus::new(
                if running {
                    ComponentPhase::Running
                } else {
                    ComponentPhase::Stopped
                },
                None,
            ),
            tun: ComponentStatus::new(
                if running {
                    ComponentPhase::Running
                } else {
                    ComponentPhase::Stopped
                },
                Some("demo-tun".into()),
            ),
            dns: ComponentStatus::new(
                if running {
                    ComponentPhase::Running
                } else {
                    ComponentPhase::Stopped
                },
                None,
            ),
            providers: ProviderSummary::default(),
        }
    }

    async fn helper_status(&self) -> Result<HelperStatus, CoreError> {
        Ok(HelperStatus {
            available: true,
            authorized: true,
            version: Some("demo".into()),
        })
    }
    async fn ensure_hiddify(&self, _cancel: CancellationToken) -> Result<(), CoreError> {
        tokio::time::sleep(Duration::from_millis(100)).await;
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
        *self.running.lock().await = true;
        Ok(())
    }
    async fn stop_core(&self) -> Result<(), CoreError> {
        *self.running.lock().await = false;
        Ok(())
    }
    async fn core_process(&self) -> Result<ProcessStatus, CoreError> {
        Ok(ProcessStatus {
            running: *self.running.lock().await,
            pid: Some(1234),
        })
    }
    async fn tun_status(&self) -> Result<TunStatus, CoreError> {
        Ok(TunStatus {
            active: *self.running.lock().await,
            name: Some("demo-tun".into()),
        })
    }
    async fn check_readiness(
        &self,
        _cancel: CancellationToken,
    ) -> Result<ReadinessReport, CoreError> {
        Ok(ReadinessReport {
            controller_ready: true,
            egress_ready: true,
            providers: ProviderSummary {
                ready: 3,
                total: 3,
                rules_loaded: 100,
                last_refresh: Some(chrono::Utc::now()),
            },
            exit_ip: Some("203.0.113.42".into()),
        })
    }
    async fn cleanup_owned_state(&self) -> Result<CleanupReport, CoreError> {
        *self.running.lock().await = false;
        Ok(CleanupReport {
            process_stopped: true,
            tun_removed: true,
            dns_restored: true,
            routes_removed: 0,
            warnings: vec![],
        })
    }

    async fn clear_hiddify_system_proxy(&self) -> Result<bool, CoreError> {
        Ok(false)
    }

    async fn restore_hiddify_system_proxy(&self) -> Result<(), CoreError> {
        Ok(())
    }
}
