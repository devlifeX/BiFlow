[CmdletBinding()]
param(
    [ValidateSet("windows")]
    [string]$Target = "windows",
    [ValidateSet("compile", "nsis", "collect")]
    [string]$From,
    [switch]$Force,
    [switch]$Help
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ProjectDir = (Resolve-Path $PSScriptRoot).Path
$TargetDir = if ($env:CARGO_TARGET_DIR) {
    if ([IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
    } else {
        [IO.Path]::GetFullPath((Join-Path $ProjectDir $env:CARGO_TARGET_DIR))
    }
} else {
    Join-Path $ProjectDir "target"
}
$CrateBin = "iran-split-desktop"
$BuildVersion = ""
$NodeVersion = 24
$PnpmVersion = "9.0.1"

function Fail([string]$Message) {
    throw $Message
}

function Log([string]$Message) {
    Write-Host $Message
}

function Require-Command([string]$Name, [string]$InstallHint) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        Fail "$Name is required. $InstallHint"
    }
}

function Invoke-Tool([string]$FilePath, [string[]]$Arguments) {
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        Fail "$FilePath exited with code $LASTEXITCODE"
    }
}

function Get-Plan([string]$Key) {
    $output = & node (Join-Path $ProjectDir "scripts/build-plan.mjs") $Key
    if ($LASTEXITCODE -ne 0) {
        Fail "build-plan.mjs failed for '$Key'"
    }
    return ([string]::Join("`n", $output)).Trim()
}

function Assert-BuildVersion {
    $current = Get-Plan "version"
    if ([string]::IsNullOrWhiteSpace($BuildVersion) -or $current -ne $BuildVersion) {
        Fail "version changed during the build: started with '$BuildVersion', now '$current'"
    }
}

function Windows-InstallerName {
    return "BiFlow_${BuildVersion}_x64-setup.exe"
}

function Windows-Prefix {
    return Join-Path $TargetDir "release"
}

function Windows-PortablePath {
    return Join-Path (Windows-Prefix) "$CrateBin.exe"
}

function Windows-InstallerPath {
    return Join-Path (Windows-Prefix) (Join-Path "bundle/nsis" (Windows-InstallerName))
}

function StampDir {
    return Join-Path $TargetDir "biflow-build"
}

function StampPath([string]$Stage) {
    return Join-Path (StampDir) "$Stage.stamp"
}

function Write-Stamp([string]$Stage) {
    New-Item -ItemType Directory -Force -Path (StampDir) | Out-Null
    [IO.File]::WriteAllText((StampPath $Stage), "$BuildVersion`n")
}

function Test-CurrentStamp([string]$Stage) {
    $path = StampPath $Stage
    return (Test-Path -LiteralPath $path) -and ((Get-Content -Raw -LiteralPath $path).Trim() -eq $BuildVersion)
}

function Test-StageDone([string]$Stage) {
    switch ($Stage) {
        "compile" {
            if (-not (Test-Path -LiteralPath (Windows-PortablePath))) { return $false }
            if (Test-CurrentStamp "windows-compile") { return $true }
            return (Test-StageDone "nsis")
        }
        "nsis" {
            return Test-Path -LiteralPath (Windows-InstallerPath)
        }
        "collect" {
            $layout = Get-Plan "windows.dir"
            $root = Join-Path $ProjectDir $layout
            return (Test-Path -LiteralPath (Join-Path $root "BiFlow.exe")) -and
                (Test-Path -LiteralPath (Join-Path $root (Windows-InstallerName)))
        }
        default { return $false }
    }
}

function Should-RunStage([string]$Stage) {
    if ($Force) { return $true }
    $stages = @("compile", "nsis", "collect")
    if ($From) {
        $fromIndex = [Array]::IndexOf($stages, $From.ToLowerInvariant())
        $stageIndex = [Array]::IndexOf($stages, $Stage)
        if ($fromIndex -ge 0) { return $stageIndex -ge $fromIndex }
    }
    return -not (Test-StageDone $Stage)
}

function Ensure-Node {
    Require-Command "node" "Install Node.js $NodeVersion or newer and run this script again."
    $major = [int]((& node -p "process.versions.node.split('.')[0]").Trim())
    if ($major -lt $NodeVersion) {
        Fail "Node.js $NodeVersion or newer is required; found $major."
    }
    Log "Node.js $((& node --version).Trim()) is ready"
}

function Refresh-ToolPath {
    $cargoBin = Join-Path $env:USERPROFILE ".cargo/bin"
    if ((Test-Path -LiteralPath $cargoBin) -and ($env:Path -notlike "*$cargoBin*")) {
        $env:Path = "$cargoBin;$env:Path"
    }
}

$script:PnpmLaunch = @("pnpm")

function Invoke-Pnpm([string[]]$Arguments) {
    $file = $script:PnpmLaunch[0]
    $prefix = @()
    if ($script:PnpmLaunch.Count -gt 1) {
        $prefix = $script:PnpmLaunch[1..($script:PnpmLaunch.Count - 1)]
    }
    Invoke-Tool $file @($prefix + $Arguments)
}

function Ensure-Pnpm {
    # `corepack enable` writes pnpm next to node.exe under Program Files and
    # fails with EPERM for a normal user. `corepack pnpm` uses the user cache.
    $env:COREPACK_ENABLE_DOWNLOAD_PROMPT = "0"
    $pnpmWorks = $false
    if (Get-Command pnpm -ErrorAction SilentlyContinue) {
        & pnpm --version | Out-Null
        $pnpmWorks = $LASTEXITCODE -eq 0
    }
    if ($pnpmWorks) {
        $script:PnpmLaunch = @("pnpm")
    } elseif (Get-Command corepack -ErrorAction SilentlyContinue) {
        Invoke-Tool "corepack" @("prepare", "pnpm@$PnpmVersion")
        $script:PnpmLaunch = @("corepack", "pnpm")
    } else {
        Fail "pnpm is required. Install pnpm $PnpmVersion or use the Node.js Corepack that ships with it."
    }
    $file = $script:PnpmLaunch[0]
    $prefix = @()
    if ($script:PnpmLaunch.Count -gt 1) {
        $prefix = $script:PnpmLaunch[1..($script:PnpmLaunch.Count - 1)]
    }
    $version = (& $file @prefix --version)
    if ($LASTEXITCODE -ne 0) {
        Fail "$file exited with code $LASTEXITCODE"
    }
    Log "pnpm $($version.Trim()) is ready"
}

function Ensure-NodeModules {
    if (-not (Test-Path -LiteralPath (Join-Path $ProjectDir "node_modules"))) {
        Log "Installing pinned frontend dependencies..."
        Push-Location $ProjectDir
        try { Invoke-Pnpm @("install", "--frozen-lockfile") }
        finally { Pop-Location }
    }
}

function Ensure-Rust {
    Require-Command "cargo" "Install Rust from rust-toolchain.toml and run this script again."
    Require-Command "rustc" "Install Rust from rust-toolchain.toml and run this script again."
    $toolchain = (Select-String -Path (Join-Path $ProjectDir "rust-toolchain.toml") -Pattern '^channel = "([^"]+)"').Matches[0].Groups[1].Value
    $actual = ((& rustc --version) -split " ")[1]
    if ($actual -ne $toolchain) {
        Fail "Rust $toolchain is required; found $actual."
    }
    Log "Rust $actual is ready"
}

function Ensure-Nsis {
    $makensis = Get-Command "makensis" -ErrorAction SilentlyContinue
    if (-not $makensis) {
        $candidates = @(
            (Join-Path ${env:ProgramFiles(x86)} "NSIS/makensis.exe"),
            (Join-Path $env:ProgramFiles "NSIS/makensis.exe")
        )
        foreach ($candidate in $candidates) {
            if ($candidate -and (Test-Path -LiteralPath $candidate)) {
                $env:Path = "$(Split-Path -Parent $candidate);$env:Path"
                $makensis = Get-Command "makensis" -ErrorAction SilentlyContinue
                break
            }
        }
    }
    if (-not $makensis) {
        Fail "NSIS (makensis.exe) is required. Install NSIS and run this script again."
    }
    Log "NSIS is ready"
}

function Ensure-Requirements {
    Log "Checking Windows build requirements..."
    Refresh-ToolPath
    Ensure-Node
    Ensure-Pnpm
    Ensure-NodeModules
    Ensure-Rust
    Ensure-Nsis
    Log "All Windows build requirements are ready"
}

function Stage-WindowsHelper {
    $staged = Join-Path $ProjectDir "packaging/staged"
    New-Item -ItemType Directory -Force -Path $staged | Out-Null
    Log "Building the privileged Windows helper..."
    Push-Location $ProjectDir
    try { Invoke-Tool "cargo" @("build", "--release", "-p", "iran-split-helper") }
    finally { Pop-Location }
    $source = Join-Path (Windows-Prefix) "iran-split-helper.exe"
    if (-not (Test-Path -LiteralPath $source)) {
        Fail "Windows helper build did not produce $source"
    }
    Copy-Item -Force -LiteralPath $source -Destination (Join-Path $staged "iran-split-helper.exe")
    Log "Staged $staged\iran-split-helper.exe"
}

function Test-FrontendDist {
    return Test-Path -LiteralPath (Join-Path $ProjectDir "apps/desktop/dist/index.html")
}

function Get-SigningConfig {
    if ($env:TAURI_SIGNING_PRIVATE_KEY) { return $null }
    Log "TAURI_SIGNING_PRIVATE_KEY is unset; building unsigned local packages"
    return '{"bundle":{"createUpdaterArtifacts":false}}'
}

function New-ConfigFile([string]$Json) {
    $path = Join-Path ([IO.Path]::GetTempPath()) "biflow-tauri-$([Guid]::NewGuid().ToString('N')).json"
    [IO.File]::WriteAllText($path, $Json, (New-Object Text.UTF8Encoding($false)))
    return $path
}

function Invoke-TauriBuild([bool]$SkipFrontend, [string[]]$TauriArgs) {
    $configArgs = @()
    $configFiles = @()
    try {
        $signingConfig = Get-SigningConfig
        if ($signingConfig) {
            $configFile = New-ConfigFile $signingConfig
            $configFiles += $configFile
            $configArgs += @("--config", $configFile)
        }
        if ($SkipFrontend) {
            if (-not (Test-FrontendDist)) {
                Fail "Cannot skip the frontend build: apps/desktop/dist/index.html is missing"
            }
            $configFile = New-ConfigFile '{"build":{"beforeBuildCommand":""}}'
            $configFiles += $configFile
            $configArgs += @("--config", $configFile)
        }
        $args = @("tauri", "build") + $TauriArgs + $configArgs
        Push-Location $ProjectDir
        try { Invoke-Pnpm $args }
        finally { Pop-Location }
    }
    finally {
        foreach ($configFile in $configFiles) {
            Remove-Item -Force -LiteralPath $configFile -ErrorAction SilentlyContinue
        }
    }
}

function Copy-Artifact([string]$Source, [string]$Destination) {
    if (-not (Test-Path -LiteralPath $Source)) {
        Fail "Expected artifact is missing: $Source"
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Destination) | Out-Null
    Copy-Item -Force -LiteralPath $Source -Destination $Destination
    Log "Wrote $Destination"
}

function Collect-Windows {
    Assert-BuildVersion
    $artifactDir = Join-Path $ProjectDir (Get-Plan "windows.dir")
    Copy-Artifact (Windows-PortablePath) (Join-Path $artifactDir (Get-Plan "windows.exe"))
    Copy-Artifact (Windows-InstallerPath) (Join-Path $artifactDir (Windows-InstallerName))

    $rules = Join-Path $artifactDir "rules"
    New-Item -ItemType Directory -Force -Path $rules | Out-Null
    Copy-Item -Force -Recurse -Path (Join-Path $ProjectDir "resources/rules/*") -Destination $rules

    $dependencies = Join-Path $artifactDir "dependencies"
    New-Item -ItemType Directory -Force -Path $dependencies | Out-Null
    foreach ($file in @(
        (Join-Path $ProjectDir "vendor/mihomo/windows-x86_64/mihomo.exe"),
        (Join-Path $ProjectDir "vendor/wintun/windows-x86_64/wintun.dll")
    )) {
        if (Test-Path -LiteralPath $file) {
            Copy-Item -Force -LiteralPath $file -Destination $dependencies
        }
    }

    $stagedHelper = Join-Path $ProjectDir "packaging/staged/iran-split-helper.exe"
    if (Test-Path -LiteralPath $stagedHelper) {
        $helperDir = Join-Path $artifactDir "helper"
        New-Item -ItemType Directory -Force -Path $helperDir | Out-Null
        Copy-Item -Force -LiteralPath $stagedHelper -Destination (Join-Path $helperDir "iran-split-helper.exe")
    }
}

function Build-Windows {
    Assert-BuildVersion
    $frontendModeSkip = $false

    if (Test-FrontendDist -and -not (Should-RunStage "compile")) {
        $frontendModeSkip = $true
    }

    if (Should-RunStage "compile") {
        Log "Stage compile: Windows app for BiFlow $BuildVersion"
        Stage-WindowsHelper
        Invoke-TauriBuild $false @("--no-bundle")
        if (-not (Test-Path -LiteralPath (Windows-PortablePath))) {
            Fail "Compile did not produce $(Windows-PortablePath)"
        }
        Write-Stamp "windows-compile"
        $frontendModeSkip = $true
    } else {
        Log "Skipping compile; already have $(Windows-PortablePath)"
    }

    if (Should-RunStage "nsis") {
        Log "Stage nsis: $(Windows-InstallerName)"
        $stagedHelper = Join-Path $ProjectDir "packaging/staged/iran-split-helper.exe"
        if (-not (Test-Path -LiteralPath $stagedHelper)) { Stage-WindowsHelper }
        Invoke-TauriBuild $frontendModeSkip @("--bundles", "nsis")
        if (-not (Test-Path -LiteralPath (Windows-InstallerPath))) {
            Fail "NSIS build did not produce $(Windows-InstallerPath)"
        }
        Write-Stamp "windows-nsis"
    } else {
        Log "Skipping nsis; installer already built"
    }

    if (Should-RunStage "collect") {
        Log "Stage collect: copying Windows artifacts"
        Collect-Windows
        Write-Stamp "windows-collect"
    } else {
        Log "Skipping collect; Windows artifacts already in $(Get-Plan 'windows.dir')"
    }
}

function Show-Summary {
    Assert-BuildVersion
    $artifactDir = Join-Path $ProjectDir (Get-Plan "windows.dir")
    Log ""
    Log "BiFlow $BuildVersion Windows artifacts:"
    foreach ($file in @(
        (Join-Path $artifactDir (Get-Plan "windows.exe")),
        (Join-Path $artifactDir (Windows-InstallerName))
    )) {
        if (Test-Path -LiteralPath $file) { Log "  $file" }
    }
}

if ($Help) {
    @"
BiFlow Windows release builder

Usage:
  .\build.ps1
  .\build.ps1 -From compile|nsis|collect
  .\build.ps1 -Force

Builds only the Windows portable executable and NSIS installer.
The version is read from the root version file.
"@ | Write-Host
    exit 0
}

if ($Target -ne "windows") { Fail "Only the Windows target is supported by build.ps1" }
if ($From -and @("compile", "nsis", "collect") -notcontains $From.ToLowerInvariant()) {
    Fail "-From must be compile, nsis, or collect"
}

Ensure-Requirements
Push-Location $ProjectDir
try { Invoke-Pnpm @("version:sync") }
finally { Pop-Location }
$BuildVersion = Get-Plan "version"
Assert-BuildVersion
Build-Windows
Show-Summary
