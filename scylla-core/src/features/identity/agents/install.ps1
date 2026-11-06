# Installs the Scylla agent on Windows, joins its App, and starts it as the service ScyllaAgent.
#
#   & ([scriptblock]::Create((irm <server>/install.ps1))) -JoinCode <join-code> -Url <server> [options]
#   & ([scriptblock]::Create((irm <server>/install.ps1))) -Update
#   & ([scriptblock]::Create((irm <server>/install.ps1))) -Uninstall
#
# Options:
#   -Url <url>                 the address of the server for this machine
#   -Capacity <n>              the largest number of jobs at the same time (default 1)
#   -ContainerRuntime <name>   auto, docker, podman, or none (default auto)
#   -NoService                 no administrator and no service: all files for this user
#   -Binary <file>             use this binary and download nothing
#   -Force                     replace the file of a machine that joined before, with a new key
#
# The service needs PowerShell as administrator. SCYLLA_AGENT_DOWNLOADS_URL replaces the folder
# of the downloads.
param(
    [string]$JoinCode = '',
    [string]$Url = '',
    [ValidateRange(1, 10000)][int]$Capacity = 1,
    [ValidatePattern('^[a-z0-9_-]+$')][string]$ContainerRuntime = 'auto',
    [switch]$NoService,
    [string]$Binary = '',
    [switch]$Update,
    [switch]$Uninstall,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$ScyllaUrl = '@SCYLLA_URL@'
$Version = '@SCYLLA_VERSION@'
$Downloads = '@DOWNLOADS_URL@'
if ($env:SCYLLA_AGENT_DOWNLOADS_URL) { $Downloads = $env:SCYLLA_AGENT_DOWNLOADS_URL }
$ServiceName = 'ScyllaAgent'
$ServiceAccount = "NT SERVICE\$ServiceName"
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$installed = Get-Service $ServiceName -ErrorAction SilentlyContinue
$userFolder = Join-Path $env:LOCALAPPDATA 'scylla-agent'
if (($Update -or $Uninstall) -and -not $installed -and (Test-Path (Join-Path $userFolder 'agent.json'))) {
    $NoService = $true
}
if ($NoService) {
    $bin = Join-Path $env:LOCALAPPDATA 'Programs\scylla-agent'
    $data = $userFolder
} else {
    $bin = Join-Path $env:ProgramFiles 'Scylla Agent'
    $data = Join-Path $env:ProgramData 'scylla-agent'
    $principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'run PowerShell as administrator for the service, or add -NoService'
    }
}
$exe = Join-Path $bin 'scylla-agent.exe'
$config = Join-Path $data 'agent.json'
$work = Join-Path $data 'work'

function Get-Binary([string]$Folder) {
    $file = Join-Path $Folder 'scylla-agent.exe'
    if ($Binary) {
        if (-not (Test-Path $Binary -PathType Leaf)) { throw "$Binary does not exist" }
        Copy-Item $Binary $file
    } else {
        $target = switch ($env:PROCESSOR_ARCHITECTURE) {
            'AMD64' { 'x86_64-pc-windows-msvc' }
            'ARM64' { 'aarch64-pc-windows-msvc' }
            default { throw "no build for the CPU $env:PROCESSOR_ARCHITECTURE" }
        }
        $archive = "scylla-agent-$target.zip"
        Write-Host "downloading scylla-agent $Version for $target"
        Invoke-WebRequest "$Downloads/$archive" -OutFile (Join-Path $Folder $archive) -UseBasicParsing
        Invoke-WebRequest "$Downloads/SHA256SUMS" -OutFile (Join-Path $Folder 'SHA256SUMS') -UseBasicParsing
        $expected = Get-Content (Join-Path $Folder 'SHA256SUMS') | ForEach-Object {
            $hash, $name = $_ -split '\s+', 2
            if ($name -eq $archive -or $name -eq "*$archive") { $hash }
        } | Select-Object -First 1
        if (-not $expected) { throw "no checksum for $archive" }
        $actual = (Get-FileHash (Join-Path $Folder $archive) -Algorithm SHA256).Hash
        if ($actual -ne $expected) { throw "the checksum of $archive does not match" }
        Expand-Archive (Join-Path $Folder $archive) -DestinationPath $Folder -Force
    }
    $found = & $file --version
    if ($found -ne "scylla-agent $Version") {
        Write-Warning "this is $found, and the server has the version $Version"
    }
    return $file
}

function Find-GitBin {
    $folders = @()
    $git = Get-Command git.exe -ErrorAction SilentlyContinue
    if ($git) { $folders += Join-Path (Split-Path (Split-Path $git.Source)) 'bin' }
    $folders += Join-Path $env:ProgramFiles 'Git\bin'
    foreach ($folder in $folders) {
        if (Test-Path (Join-Path $folder 'sh.exe')) { return $folder }
    }
    return $null
}

function Set-ServicePath {
    $key = "HKLM:\SYSTEM\CurrentControlSet\Services\$ServiceName"
    $gitBin = Find-GitBin
    if ($gitBin) {
        $machine = [Environment]::GetEnvironmentVariable('PATH', 'Machine')
        New-ItemProperty $key -Name Environment -PropertyType MultiString -Value @("PATH=$gitBin;$machine") -Force | Out-Null
    } else {
        Remove-ItemProperty $key -Name Environment -ErrorAction SilentlyContinue
        Write-Warning 'a script step needs sh, and this machine has no Git for Windows. Install it with: winget install --id Git.Git -e'
    }
}

function Set-ServiceAccount {
    sc.exe config $ServiceName obj= $ServiceAccount | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "the account $ServiceAccount cannot run the service" }
    icacls $data /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' "${ServiceAccount}:(OI)(CI)M" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "the access of $data cannot change" }
}

function Install-Agent {
    if (-not $JoinCode) { throw 'give the join code of the app: -JoinCode <join-code> -Url <server>' }
    $address = if ($Url) { $Url } else { $ScyllaUrl }
    if ($address -notmatch '^https?://') { throw '-Url needs an address with http:// or https://' }
    if ((Test-Path $config) -and -not $Force) {
        throw "$config exists, so this machine joined before. -Update replaces the binary and keeps the key. -Force makes a new key"
    }
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('scylla-agent-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $file = Get-Binary $tmp
        $joined = Join-Path $tmp 'agent.json'
        & $file --config $joined join $address $JoinCode | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'the join failed, and this machine did not change' }
        Write-Host "the machine joined $address"
        if (Test-Path $config) { Write-Host 'the old key stays on the App until you revoke it' }

        if ($installed) { Stop-Service $ServiceName -Force }
        New-Item -ItemType Directory -Force -Path $bin, $data, $work | Out-Null
        Copy-Item $file $exe -Force
        Copy-Item $joined $config -Force
    } finally {
        Remove-Item -Recurse -Force $tmp
    }

    $arguments = @('--config', "`"$config`"", 'run', '--workspace-root', "`"$work`"", '--capacity', $Capacity)
    if ($ContainerRuntime -ne 'auto') { $arguments += @('--container-runtime', $ContainerRuntime) }
    if ($NoService) {
        Write-Host "start the agent with: & `"$exe`" $($arguments -join ' ')"
        $gitBin = Find-GitBin
        if (-not (Get-Command sh.exe -ErrorAction SilentlyContinue)) {
            if ($gitBin) { Write-Host "a script step needs sh: add $gitBin to the PATH first" }
            else { Write-Warning 'a script step needs sh. Install Git for Windows with: winget install --id Git.Git -e' }
        }
        return
    }

    $command = "`"$exe`" $(($arguments + '--service') -join ' ')"
    if ($installed) {
        Set-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Services\$ServiceName" -Name ImagePath -Value $command
    } else {
        New-Service -Name $ServiceName -DisplayName 'Scylla agent' -Description 'Runs the jobs of Scylla.' `
            -BinaryPathName $command -StartupType Automatic | Out-Null
    }
    Set-ServiceAccount
    Set-ServicePath
    sc.exe failure $ServiceName reset= 86400 actions= restart/5000/restart/5000/restart/60000 | Out-Null
    Start-Service $ServiceName
    Write-Host "the agent runs as the service $ServiceName; logs in $data\agent.*.log"
}

function Update-Agent {
    if ($JoinCode) { throw '-Update needs no join code' }
    if (-not (Test-Path $config)) { throw "$config does not exist, so this machine did not join. Give a join code" }
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('scylla-agent-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $file = Get-Binary $tmp
        if ($installed) { Stop-Service $ServiceName -Force }
        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        Copy-Item $file $exe -Force
    } finally {
        Remove-Item -Recurse -Force $tmp
    }
    if ($installed) {
        Start-Service $ServiceName
        Write-Host "the agent runs: $(& $exe --version)"
    } else {
        Write-Host "$exe is now $(& $exe --version). Start the agent again"
    }
}

function Uninstall-Agent {
    if ($JoinCode) { throw '-Uninstall needs no join code' }
    if ($installed) {
        Stop-Service $ServiceName -Force
        sc.exe delete $ServiceName | Out-Null
    }
    foreach ($folder in $bin, $data) {
        if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    }
    Write-Host 'the agent left this machine. Its key stays on the server until you revoke it or delete the agent'
}

if ($Uninstall) {
    Uninstall-Agent
} elseif ($Update) {
    Update-Agent
} else {
    Install-Agent
}
