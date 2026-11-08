//! Release only the configured Mihomo adapter, then read back native state.
use super::HelperServiceError;

// Executed by the privileged helper. Tests replace every native cmdlet with
// in-memory functions; no regression test changes the host network.
const SCRIPT: &str = r"
$ErrorActionPreference = 'Stop'
$stage = 'adapter_enumeration'
trap { [Console]::Out.WriteLine('failed:' + $stage); exit 1 }
function Owned-Adapter {
    $items = @(Get-NetAdapter -IncludeHidden -ErrorAction Stop | Where-Object { $_.Name -eq $tunName })
    if ($items.Count -gt 1) { throw 'ambiguous adapter' }
    if ($items.Count -eq 0) { return }
    $item = $items[0]
    if ($item.InterfaceDescription -notmatch '^(Meta Tunnel|Mihomo Tunnel|Wintun Userspace Tunnel)( #\d+)?$') { throw 'adapter ownership mismatch' }
    return $item
}
function Capture-Remains($item) {
    if ($null -eq $item) { return $false }
    $routes = @(Get-NetRoute -ErrorAction Stop | Where-Object { $_.InterfaceIndex -eq $item.ifIndex })
    $dns = @(Get-DnsClientServerAddress -ErrorAction Stop | Where-Object { $_.InterfaceIndex -eq $item.ifIndex } | ForEach-Object { $_.ServerAddresses })
    return ([int]$item.AdminStatus -ne 2 -or $routes.Count -gt 0 -or $dns.Count -gt 0)
}
$item = Owned-Adapter
if ($prepare -and $null -ne $item -and [int]$item.AdminStatus -eq 2) {
    $stage = 'adapter_enable'
    $item | Enable-NetAdapter -Confirm:$false -ErrorAction Stop
}
if ($release -and $null -ne $item) {
    $identity = $item.InterfaceGuid
    $current = Owned-Adapter
    if ($null -eq $current -or $current.InterfaceGuid -ne $identity) { throw 'adapter identity changed' }
    # A persistent Wintun device can survive TerminateProcess. Do not uninstall
    # its driver or touch physical adapters, DHCP, other VPNs, or global routes.
    $stage = 'route_release'
    Get-NetRoute -ErrorAction Stop | Where-Object { $_.InterfaceIndex -eq $item.ifIndex } | Remove-NetRoute -Confirm:$false -ErrorAction Stop
    $stage = 'dns_reset'
    $dns = @(Get-DnsClientServerAddress -ErrorAction Stop | Where-Object { $_.InterfaceIndex -eq $item.ifIndex } | ForEach-Object { $_.ServerAddresses })
    if ($dns.Count -gt 0) {
        Set-DnsClientServerAddress -InterfaceIndex $item.ifIndex -ResetServerAddresses -ErrorAction Stop
    }
    if ([int]$item.AdminStatus -ne 2) {
        $stage = 'adapter_disable'
        $item | Disable-NetAdapter -Confirm:$false -ErrorAction Stop
    }
}
$stage = 'native_verification'
if (Capture-Remains (Owned-Adapter)) { 'active' } else { 'clear' }
";

fn script(name: &str, release: bool, prepare: bool) -> Result<String, HelperServiceError> {
    if name.is_empty()
        || name.len() > 15
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(HelperServiceError::UnsafeConfig(
            "unsafe tunnel name".into(),
        ));
    }
    Ok(format!(
        "$tunName = '{name}'; $release = ${release}; $prepare = ${prepare};\n{SCRIPT}"
    ))
}

#[cfg(windows)]
pub(crate) async fn captured(name: &str, release: bool) -> Result<bool, HelperServiceError> {
    run(name, release, false).await
}

#[cfg(windows)]
pub(crate) async fn prepare(name: &str) -> Result<(), HelperServiceError> {
    run(name, false, true).await.map(|_| ())
}

#[cfg(windows)]
async fn run(name: &str, release: bool, prepare: bool) -> Result<bool, HelperServiceError> {
    use std::{process::Stdio, time::Duration};
    use tokio::process::Command;
    let trace_id = uuid::Uuid::new_v4();
    let script = script(name, release, prepare)?;
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x0800_0000)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .output(),
    )
    .await;
    let mut failure_stage = "native_command";
    let result = match output {
        Ok(Ok(output)) if output.status.success() => {
            match String::from_utf8_lossy(&output.stdout).trim() {
                "active" => Ok(true),
                "clear" => Ok(false),
                _ => Err(HelperServiceError::Process(
                    "Windows network verification returned invalid state".into(),
                )),
            }
        }
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            failure_stage = match stdout.trim() {
                "failed:adapter_enumeration" => "adapter_enumeration",
                "failed:adapter_enable" => "adapter_enable",
                "failed:route_release" => "route_release",
                "failed:dns_reset" => "dns_reset",
                "failed:adapter_disable" => "adapter_disable",
                "failed:native_verification" => "native_verification",
                _ => "native_command",
            };
            Err(HelperServiceError::Process(format!(
                "Windows owned network {failure_stage} failed"
            )))
        }
        _ => Err(HelperServiceError::Process(
            "Windows owned network command failed or timed out".into(),
        )),
    };
    tracing::info!(event = "helper.windows_network_checked", section = "helper_network",
        initiator = "helper_process", cause = "native_adapter_routes_dns_check", %trace_id,
        release, prepare, failure_stage, verified = result.is_ok(), captured = result.as_ref().ok().copied(),
        trace_route = "helper_process->windows_network->native_state",
        "checked Windows capture without logging network settings");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_names_cannot_become_powershell() {
        for name in ["", "*", "tun'; exit", "a/b", "1234567890123456"] {
            assert!(script(name, true, false).is_err());
        }
        let text = script("biflow-test", true, false).unwrap();
        assert!(!text.contains("ifconfig"));
        assert!(text.contains("Get-NetAdapter -IncludeHidden -ErrorAction Stop"));
    }

    #[cfg(windows)]
    #[test]
    fn native_errors_report_only_the_safe_stage() {
        let production = script("biflow-test", false, false).unwrap();
        let fixture =
            format!("function Get-NetAdapter {{ throw 'private fixture payload' }}; {production}");
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &fixture])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "failed:adapter_enumeration"
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private fixture payload"));
    }

    #[cfg(windows)]
    #[test]
    fn native_release_is_scoped_verified_retryable_and_idempotent() {
        // Every OS command in SCRIPT is shadowed before production code runs.
        let fixture = r"
$script:adapter = [pscustomobject]@{ Name='biflow-test'; InterfaceDescription='Meta Tunnel'; ifIndex=42; InterfaceGuid='fixture'; AdminStatus=1 }
$script:routes = 2
$script:dns = @('198.18.0.2')
$script:failDns = $true
$script:ignoreDisable = $false
function Get-NetAdapter { param([switch]$IncludeHidden, $ErrorAction) $script:adapter }
function Get-NetRoute { param($ErrorAction) for ($i=0; $i -lt $script:routes; $i++) { [pscustomobject]@{InterfaceIndex=42;Index=$i} }; [pscustomobject]@{InterfaceIndex=99;Index=99} }
function Remove-NetRoute { param([Parameter(ValueFromPipeline=$true)]$InputObject, $Confirm) process { if ($InputObject.InterfaceIndex -ne 42) { throw 'touched other route' }; $script:routes = 0 } }
function Get-DnsClientServerAddress { param($ErrorAction) [pscustomobject]@{ InterfaceIndex=42; ServerAddresses=$script:dns }; [pscustomobject]@{InterfaceIndex=99; ServerAddresses=@('192.0.2.53')} }
function Set-DnsClientServerAddress { param($InterfaceIndex, [switch]$ResetServerAddresses, $ErrorAction) if ($script:dns.Count -eq 0) { throw 'DNS interface already absent' }; if ($script:failDns) { throw 'fixture failure' }; if ($InterfaceIndex -ne 42 -or !$ResetServerAddresses) { throw 'wrong DNS reset' }; $script:dns=@() }
function Disable-NetAdapter { param([Parameter(ValueFromPipeline=$true)]$InputObject, $Confirm) process { if ($InputObject.ifIndex -ne 42) { throw 'wrong adapter' }; if ([int]$script:adapter.AdminStatus -eq 2) { throw 'already disabled' }; if (!$script:ignoreDisable) { $script:adapter.AdminStatus=2 } } }
function Enable-NetAdapter { param([Parameter(ValueFromPipeline=$true)]$InputObject, $Confirm) process { if ($InputObject.ifIndex -ne 42) { throw 'wrong adapter' }; $script:adapter.AdminStatus=1 } }
";
        let production = script("biflow-test", true, false).unwrap().replace(
            "trap { [Console]::Out.WriteLine('failed:' + $stage); exit 1 }",
            "",
        );
        let probe = script("biflow-test", false, false).unwrap().replace(
            "trap { [Console]::Out.WriteLine('failed:' + $stage); exit 1 }",
            "",
        );
        let prepare = script("biflow-test", false, true).unwrap().replace(
            "trap { [Console]::Out.WriteLine('failed:' + $stage); exit 1 }",
            "",
        );
        let test = format!(
            "{fixture}\n$action = {{ {production} }}; $probe = {{ {probe} }}; $prepare = {{ {prepare} }};\n\
             if ((& $probe) -ne 'active') {{ throw 'missed controller-independent capture' }};\n\
             $failed=$false; try {{ & $action }} catch {{ $failed=$true }}; if (!$failed) {{ throw 'ignored DNS failure' }};\n\
             $script:failDns=$false; $script:ignoreDisable=$true; if ((& $action) -ne 'active') {{ throw 'trusted failed disable' }};\n\
             $script:ignoreDisable=$false; if ((& $action) -ne 'clear') {{ throw 'retry failed' }};\n\
             if ((& $action) -ne 'clear') {{ throw 'not idempotent' }};\n\
             $script:adapter.InterfaceDescription='Meta Tunnel #2'; if ((& $action) -ne 'clear') {{ throw 'driver suffix rejected' }};\n\
             if ((& $prepare) -ne 'active') {{ throw 'adapter not reenabled for reconnect' }};\n\
             if ((& $action) -ne 'clear') {{ throw 'second release failed' }};\n\
             $script:adapter.InterfaceDescription='Physical Ethernet'; $failed=$false; try {{ & $action }} catch {{ $failed=$true }}; if (!$failed) {{ throw 'touched physical adapter' }};\n\
             $script:adapter=$null; if ((& $action) -ne 'clear') {{ throw 'absent adapter failed' }};\n\
             function Get-NetAdapter {{ throw 'enumeration failed' }}; $failed=$false; try {{ & $probe }} catch {{ $failed=$true }}; if (!$failed) {{ throw 'inspection failure counted as clear' }}"
        );
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &test])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
