# Installs the Scylla agent on Windows, joins its App, and starts it as the service ScyllaAgent.
#
#   & ([scriptblock]::Create((irm <server>/install.ps1))) -JoinCode <join-code> [options]
#
# Options:
#   -Capacity <n>              the largest number of jobs at the same time (default 1)
#   -ContainerRuntime <name>   docker, podman, or none (default: docker or podman if found)
#   -NoService                 install for this user and join only; print the run command
#
# The service needs PowerShell as administrator. SCYLLA_AGENT_DOWNLOADS_URL replaces the folder
# of the downloads.
param(
    [Parameter(Mandatory = $true)][string]$JoinCode,
    [ValidateRange(1, 10000)][int]$Capacity = 1,
    [ValidatePattern('^[a-z0-9_-]*$')][string]$ContainerRuntime = '',
    [switch]$NoService
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$ScyllaUrl = '@SCYLLA_URL@'
$Version = '@SCYLLA_VERSION@'
$Downloads = '@DOWNLOADS_URL@'
if ($env:SCYLLA_AGENT_DOWNLOADS_URL) { $Downloads = $env:SCYLLA_AGENT_DOWNLOADS_URL }
$ServiceName = 'ScyllaAgent'

$target = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x86_64-pc-windows-msvc' }
    'ARM64' { 'aarch64-pc-windows-msvc' }
    default { throw "no build for the CPU $env:PROCESSOR_ARCHITECTURE" }
}

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
$admin = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $NoService -and -not $admin) {
    throw 'run PowerShell as administrator for the service, or add -NoService'
}
if ($NoService) {
    $bin = Join-Path $env:LOCALAPPDATA 'Programs\scylla-agent'
    $data = Join-Path $env:LOCALAPPDATA 'scylla-agent'
} else {
    $bin = Join-Path $env:ProgramFiles 'Scylla Agent'
    $data = Join-Path $env:ProgramData 'scylla-agent'
}
$exe = Join-Path $bin 'scylla-agent.exe'
$config = Join-Path $data 'agent.json'
$work = Join-Path $data 'work'

if (-not $ContainerRuntime) {
    $ContainerRuntime = 'none'
    foreach ($candidate in 'docker', 'podman') {
        if (Get-Command $candidate -ErrorAction SilentlyContinue) { $ContainerRuntime = $candidate; break }
    }
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ('scylla-agent-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $archive = "scylla-agent-$target.zip"
    Write-Host "downloading scylla-agent $Version for $target"
    Invoke-WebRequest "$Downloads/$archive" -OutFile (Join-Path $tmp $archive) -UseBasicParsing
    Invoke-WebRequest "$Downloads/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS') -UseBasicParsing
    $expected = Get-Content (Join-Path $tmp 'SHA256SUMS') | ForEach-Object {
        $hash, $file = $_ -split '\s+', 2
        if ($file -eq $archive -or $file -eq "*$archive") { $hash }
    } | Select-Object -First 1
    if (-not $expected) { throw "no checksum for $archive" }
    $actual = (Get-FileHash (Join-Path $tmp $archive) -Algorithm SHA256).Hash
    if ($actual -ne $expected) { throw "the checksum of $archive does not match" }

    $service = Get-Service $ServiceName -ErrorAction SilentlyContinue
    if (-not $NoService -and $service) { Stop-Service $ServiceName -Force }
    New-Item -ItemType Directory -Force -Path $bin, $data, $work | Out-Null
    Expand-Archive (Join-Path $tmp $archive) -DestinationPath $bin -Force
} finally {
    Remove-Item -Recurse -Force $tmp
}

if (-not $NoService) {
    icacls $data /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "the access of $data cannot change" }
}

& $exe --config $config join $ScyllaUrl $JoinCode
if ($LASTEXITCODE -ne 0) { throw 'the join failed' }

$arguments = @('--config', "`"$config`"", 'run', '--workspace-root', "`"$work`"", '--capacity', $Capacity)
if ($ContainerRuntime -ne 'none') { $arguments += @('--container-runtime', $ContainerRuntime) }

if ($NoService) {
    Write-Host "start the agent with: & `"$exe`" $($arguments -join ' ')"
    return
}

$command = "`"$exe`" $(($arguments + '--service') -join ' ')"
if ($service) {
    Set-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Services\$ServiceName" -Name ImagePath -Value $command
} else {
    New-Service -Name $ServiceName -DisplayName 'Scylla agent' -Description 'Runs the jobs of Scylla.' `
        -BinaryPathName $command -StartupType Automatic | Out-Null
}
sc.exe failure $ServiceName reset= 86400 actions= restart/5000/restart/5000/restart/60000 | Out-Null
Start-Service $ServiceName
Write-Host "the agent runs as the service $ServiceName; logs in $data\agent.*.log"
