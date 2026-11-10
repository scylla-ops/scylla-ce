# Installs the Scylla agent on Windows, joins its App, and starts it. In PowerShell as administrator,
# the agent is the service ScyllaAgent. With no administrator, the agent is for this user only, and a
# task of the user starts it at each sign-in, with no window.
#
#   & ([scriptblock]::Create((irm <server>/install.ps1))) -JoinCode <join-code> -Url <server> [options]
#   & ([scriptblock]::Create((irm <server>/install.ps1))) -Update
#   & ([scriptblock]::Create((irm <server>/install.ps1))) -Uninstall
#
# Options:
#   -Url <url>                 the address of the server for this machine
#   -Capacity <n>              the largest number of jobs at the same time (default 1)
#   -ContainerRuntime <name>   auto, docker, podman, or none (default auto)
#   -CurrentUser               for this user only, with the task, also as administrator
#   -NoService                 no service and no task: all files for this user
#   -Binary <file>             use this binary and download nothing
#   -Force                     replace the file of a machine that joined before, with a new key
#
# With no administrator, -Update and -Uninstall change the agent of this user only. As
# administrator, they also change the service. SCYLLA_AGENT_DOWNLOADS_URL replaces the folder of
# the downloads.
param(
    [string]$JoinCode = '',
    [string]$Url = '',
    [ValidateRange(1, 10000)][int]$Capacity = 1,
    [ValidatePattern('^[a-z0-9_-]+$')][string]$ContainerRuntime = 'auto',
    [switch]$CurrentUser,
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
$TaskName = "ScyllaAgent-$env:USERNAME"
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$admin = ([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$installed = Get-Service $ServiceName -ErrorAction SilentlyContinue
$task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
$serviceBin = Join-Path $env:ProgramFiles 'Scylla Agent'
$serviceData = Join-Path $env:ProgramData 'scylla-agent'
$userBin = Join-Path $env:LOCALAPPDATA 'Programs\scylla-agent'
$userData = Join-Path $env:LOCALAPPDATA 'scylla-agent'
$userExe = Join-Path $userBin 'scylla-agent.exe'
$userJoined = (Test-Path (Join-Path $userData 'agent.json')) -or [bool]$task
$conhost = Join-Path $env:SystemRoot 'System32\conhost.exe'

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

function New-TempFolder {
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('scylla-agent-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    return $tmp
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

function Write-ShellWarning {
    Write-Warning 'a script step needs sh, and this machine has no Git for Windows. Install it with: winget install --id Git.Git -e'
}

function Set-ServicePath {
    $key = "HKLM:\SYSTEM\CurrentControlSet\Services\$ServiceName"
    $gitBin = Find-GitBin
    if ($gitBin) {
        $machine = [Environment]::GetEnvironmentVariable('PATH', 'Machine')
        New-ItemProperty $key -Name Environment -PropertyType MultiString -Value @("PATH=$gitBin;$machine") -Force | Out-Null
    } else {
        Remove-ItemProperty $key -Name Environment -ErrorAction SilentlyContinue
        Write-ShellWarning
    }
}

function Set-ServiceAccount {
    sc.exe config $ServiceName obj= $ServiceAccount | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "the account $ServiceAccount cannot run the service" }
    icacls $serviceData /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' "${ServiceAccount}:(OI)(CI)M" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "the access of $serviceData cannot change" }
}

function Get-UserLaunch([string[]]$Arguments, [string]$Log) {
    $line = @('/d', '/c', 'set', 'NO_COLOR=1', '&&')
    $gitBin = Find-GitBin
    if ($gitBin) {
        $line += @('set', "`"PATH=$gitBin;%PATH%`"", '&&')
    } elseif (-not (Get-Command sh.exe -ErrorAction SilentlyContinue)) {
        Write-ShellWarning
    }
    $line += @("`"$userExe`"") + $Arguments + @('>>', "`"$Log`"", '2>&1')
    return "--headless cmd.exe $($line -join ' ')"
}

function Start-UserAgent([string]$Launch) {
    if ($admin) { Write-Warning 'this PowerShell runs as administrator, so the agent also does until the next sign-in' }
    Start-Process -FilePath $conhost -ArgumentList $Launch -WindowStyle Hidden
}

function Stop-UserAgent {
    $agents = @(Get-Process scylla-agent -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $userExe })
    if (-not $agents) { return }
    $launchers = @(Get-CimInstance Win32_Process -Filter "Name = 'cmd.exe'" |
        Where-Object { $_.CommandLine -and $_.CommandLine.Contains($userExe) } |
        ForEach-Object { Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue })
    $agents | Stop-Process -Force
    ($agents + $launchers) | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
}

function Install-UserTask([string]$Launch) {
    $action = New-ScheduledTaskAction -Execute $conhost -Argument $Launch
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $identity.User.Value
    $principal = New-ScheduledTaskPrincipal -UserId $identity.User.Value -LogonType Interactive -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero)
    Register-ScheduledTask -TaskName $TaskName -Description 'Runs the jobs of Scylla.' -Action $action -Trigger $trigger `
        -Principal $principal -Settings $settings -Force | Out-Null
}

function Remove-Folder([string]$Folder) {
    for ($attempt = 1; Test-Path $Folder; $attempt++) {
        try {
            Remove-Item -Recurse -Force $Folder
        } catch {
            if ($attempt -ge 10) { throw }
            Start-Sleep -Milliseconds 500
        }
    }
}

function Install-Agent {
    if (-not $JoinCode) { throw 'give the join code of the app: -JoinCode <join-code> -Url <server>' }
    $address = if ($Url) { $Url } else { $ScyllaUrl }
    if ($address -notmatch '^https?://') { throw '-Url needs an address with http:// or https://' }
    $forUser = $NoService -or $CurrentUser -or -not $admin
    $bin = if ($forUser) { $userBin } else { $serviceBin }
    $data = if ($forUser) { $userData } else { $serviceData }
    $exe = Join-Path $bin 'scylla-agent.exe'
    $config = Join-Path $data 'agent.json'
    $work = Join-Path $data 'work'
    if ((Test-Path $config) -and -not $Force) {
        throw "$config exists, so this machine joined before. -Update replaces the binary and keeps the key. -Force makes a new key"
    }
    $tmp = New-TempFolder
    try {
        $file = Get-Binary $tmp
        $joined = Join-Path $tmp 'agent.json'
        & $file --config $joined join $address $JoinCode | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'the join failed, and this machine did not change' }
        Write-Host "the machine joined $address"
        if (Test-Path $config) { Write-Host 'the old key stays on the App until you revoke it' }

        if ($forUser) { Stop-UserAgent } elseif ($installed) { Stop-Service $ServiceName -Force }
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
            else { Write-ShellWarning }
        }
        return
    }
    if ($forUser) {
        $log = Join-Path $data 'agent.log'
        $launch = Get-UserLaunch $arguments $log
        Install-UserTask $launch
        Start-UserAgent $launch
        Write-Host "the agent runs for this user, and the task $TaskName starts it at each sign-in; logs in $log"
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
    $service = $admin -and (Test-Path (Join-Path $serviceData 'agent.json'))
    if (-not ($userJoined -or $service)) {
        if ($installed) { throw "the agent is the service $ServiceName, so run PowerShell as administrator" }
        throw 'this machine did not join, so there is nothing to update. Give a join code'
    }

    $serviceExe = Join-Path $serviceBin 'scylla-agent.exe'
    $tmp = New-TempFolder
    try {
        $file = Get-Binary $tmp
        if ($userJoined) {
            Stop-UserAgent
            New-Item -ItemType Directory -Force -Path $userBin | Out-Null
            Copy-Item $file $userExe -Force
        }
        if ($service) {
            if ($installed) { Stop-Service $ServiceName -Force }
            New-Item -ItemType Directory -Force -Path $serviceBin | Out-Null
            Copy-Item $file $serviceExe -Force
        }
    } finally {
        Remove-Item -Recurse -Force $tmp
    }

    if ($userJoined -and $task) {
        Start-UserAgent $task.Actions[0].Arguments
        Write-Host "the agent of this user runs: $(& $userExe --version)"
    } elseif ($userJoined) {
        Write-Host "$userExe is now $(& $userExe --version). Start the agent again"
    }
    if ($service -and $installed) {
        Start-Service $ServiceName
        Write-Host "the service $ServiceName runs: $(& $serviceExe --version)"
    } elseif ($service) {
        Write-Host "$serviceExe is now $(& $serviceExe --version). Start the agent again"
    }
}

function Uninstall-Agent {
    if ($JoinCode) { throw '-Uninstall needs no join code' }
    if ($installed -and -not $admin -and -not $userJoined) {
        throw "the agent is the service $ServiceName, so run PowerShell as administrator"
    }

    if ($task) { Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false }
    Stop-UserAgent
    $folders = @($userBin, $userData)
    if ($admin) {
        if ($installed) {
            Stop-Service $ServiceName -Force
            sc.exe delete $ServiceName | Out-Null
        }
        $folders += @($serviceBin, $serviceData)
    } elseif ($installed) {
        Write-Warning "the service $ServiceName stays. Run -Uninstall in PowerShell as administrator to remove it"
    }
    foreach ($folder in $folders) { Remove-Folder $folder }
    Write-Host 'the agent left this machine. Its key stays on the server until you revoke it or delete the agent'
}

if ($Uninstall) {
    Uninstall-Agent
} elseif ($Update) {
    Update-Agent
} else {
    Install-Agent
}
