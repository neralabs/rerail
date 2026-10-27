# Runs on a throwaway Azure Windows VM for scripts/host/azure/matrix.mjs, which prepends `$RerailEnv` (a hashtable
# of settings) and scripts/host/windows-host.ps1 (the steps it shares with the release gate,
# scripts/host/windows-install-smoke.ps1), and sends the result through Azure Run Command. Windows PowerShell 5.1:
# nothing here needs PowerShell 7.
#
# As SYSTEM it provisions what a user brings (Node from its nodejs.org installer, Git for Windows from its installer,
# both machine-wide in their default folders) and the package under test, then signs the VM's ordinary user in at
# the console (automatic logon, then a restart) and runs itself again in that desktop session through a scheduled
# task that fires at the user's logon: the user's profile, the user's registry hive (where set-up writes PATH), and a
# real interactive session, as when a person opens a terminal. Run Command's own run-as-user loads no profile, and
# tasks that log on with a stored password are refused on these images. The second run installs Rerail with `npx
# @neralabs/rerail setup`, answers set-up's questions, and checks the managed runtime: launcher and `current`
# junction, daemon and native engine, PATH as a new terminal sees it, the Claude Code and Codex hooks as those agents
# spawn them, daemon restart, and removal. It uploads results.json and a log to the blob URLs it was given; the
# driver waits for them.
$Mode = if ($args.Count -gt 0) { $args[0] } else { "provision" }
$ProgressPreference = "SilentlyContinue"
$Work = "C:\rerail-env"

function Send-Blob([string]$File, [string]$Url) {
  Invoke-WebRequest -UseBasicParsing -Method Put -Uri $Url -InFile $File -Headers @{ "x-ms-blob-type" = "BlockBlob" } | Out-Null
}

# A download, retried: a new VM's first minutes on the network see failures a person would simply retry.
function Get-WithRetry([string]$Uri, [string]$OutFile = "") {
  for ($attempt = 1; ; $attempt++) {
    try {
      if ($OutFile) { Invoke-WebRequest -UseBasicParsing -ErrorAction Stop $Uri -OutFile $OutFile; return }
      return Invoke-WebRequest -UseBasicParsing -ErrorAction Stop $Uri
    } catch {
      if ($attempt -ge 5) { throw }
      Write-Host "download of $Uri failed ($($_.Exception.Message)); retrying"
      Start-Sleep ($attempt * 5)
    }
  }
}

if ($Mode -eq "provision") {
  $ErrorActionPreference = "Stop"
  New-Item -ItemType Directory -Force $Work | Out-Null
  $script = if ($PSCommandPath) { $PSCommandPath } else { $MyInvocation.MyCommand.Path }
  Copy-Item $script "$Work\guest.ps1" -Force
  $provisionLog = "$Work\provision.log"
  try {
    Start-Transcript -Path $provisionLog -Force | Out-Null
    $arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x64" }
    # The newest release of the requested major, from the installer a user downloads from nodejs.org.
    $sums = (Get-WithRetry "https://nodejs.org/dist/latest-v$($RerailEnv.Node).x/SHASUMS256.txt").Content
    $msi = ([regex]::Match($sums, "node-v[\d.]+-$arch\.msi")).Value
    if (-not $msi) { throw "no Node $($RerailEnv.Node).x installer for $arch" }
    $version = ([regex]::Match($msi, "v[\d.]+")).Value
    Get-WithRetry "https://nodejs.org/dist/$version/$msi" "$Work\$msi"
    $install = Start-Process msiexec.exe -ArgumentList "/i `"$Work\$msi`" /qn /norestart" -Wait -PassThru
    if ($install.ExitCode -ne 0) { throw "Node installer exited $($install.ExitCode)" }
    # Git for Windows, machine-wide in its default folder, as its installer puts it.
    $release = (Get-WithRetry "https://api.github.com/repos/git-for-windows/git/releases/latest").Content | ConvertFrom-Json
    $pattern = if ($arch -eq "arm64") { '^Git-[\d.]+-arm64\.exe$' } else { '^Git-[\d.]+-64-bit\.exe$' }
    $asset = $release.assets | Where-Object { $_.name -match $pattern } | Select-Object -First 1
    if ($null -eq $asset) { throw "no Git for Windows installer for $arch in $($release.tag_name)" }
    Get-WithRetry $asset.browser_download_url "$Work\$($asset.name)"
    $install = Start-Process "$Work\$($asset.name)" -ArgumentList "/VERYSILENT /NORESTART /NOCANCEL /SP- /SUPPRESSMSGBOXES" -Wait -PassThru
    if ($install.ExitCode -ne 0) { throw "Git installer exited $($install.ExitCode)" }
    if ($RerailEnv.BundleUrl) {
      Get-WithRetry $RerailEnv.BundleUrl "$Work\bundle.tar"
      New-Item -ItemType Directory -Force "$Work\bundle" | Out-Null
      & tar.exe -xf "$Work\bundle.tar" -C "$Work\bundle"
      if ($LASTEXITCODE -ne 0) { throw "could not unpack the bundle" }
    }
    Stop-Transcript | Out-Null
  } catch {
    Write-Host "provisioning failed: $_"
    Stop-Transcript | Out-Null
    Write-RerailUtf8 "$Work\results.json" (@{ env = $RerailEnv.Id; provisioned = $false; facts = @{}; steps = @() } | ConvertTo-Json -Compress)
    Send-Blob "$Work\results.json" $RerailEnv.ResultUrl
    Send-Blob $provisionLog $RerailEnv.LogUrl
    exit 1
  }

  # Sign the user in at the console on the next start, and run the checks in that session when they do.
  $winlogon = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon"
  Set-ItemProperty $winlogon AutoAdminLogon "1"
  Set-ItemProperty $winlogon DefaultUserName $RerailEnv.User
  Set-ItemProperty $winlogon DefaultPassword $RerailEnv.Password
  Set-ItemProperty $winlogon DefaultDomainName $env:COMPUTERNAME
  $user = "$env:COMPUTERNAME\$($RerailEnv.User)"
  $action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$Work\guest.ps1`" test"
  $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
  $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 45) -AllowStartIfOnBatteries
  Register-ScheduledTask -TaskName "rerail-env" -Action $action -Principal $principal -Settings $settings `
    -Trigger (New-ScheduledTaskTrigger -AtLogOn -User $user) -Force | Out-Null
  # The test run replaces this log with the full one; this copy is what is left if it never starts.
  Send-Blob $provisionLog $RerailEnv.LogUrl
  & shutdown.exe /r /t 15 /f /c "rerail-env: signing $($RerailEnv.User) in"
  Write-Host "provisioned; restarting to sign $user in"
  exit 0
}

# ---- As the user ------------------------------------------------------------------------------------------------
$ErrorActionPreference = "Continue"
$Out = Join-Path $env:USERPROFILE "rerail-env"
$Log = Join-Path $Out "test.log"
$Project = Join-Path $env:USERPROFILE "proj"
$RerailHome = Join-Path $env:USERPROFILE ".rerail"
$Launcher = Join-Path $RerailHome "bin\rerail.cmd"
New-Item -ItemType Directory -Force $Out | Out-Null
Write-RerailUtf8 $Log ""
$env:BROWSER = "none"; $env:NPM_CONFIG_UPDATE_NOTIFIER = "false"; $env:NO_COLOR = "1"
$Steps = New-Object Collections.ArrayList
$State = @{ SetupOk = $false; DaemonPort = $null; Registry = $null }

# Runs a native command, returning its combined output as one string and failing on a non-zero exit.
function Exec([string]$File, [string[]]$Arguments) {
  $output = (& $File @Arguments 2>&1 | ForEach-Object { "$_" }) -join "`n"
  if ($LASTEXITCODE -ne 0) { throw "$File $($Arguments -join ' ') exited ${LASTEXITCODE}:`n$output" }
  return $output
}

# Runs a step, keeping each line it prints as it prints it, so a step that throws keeps what it printed before.
function Step([string]$Name, [scriptblock]$Body) {
  Add-Content -Path $Log -Value "`n==== $Name"
  $started = Get-Date
  $ok = $true
  $lines = New-Object Collections.Generic.List[string]
  try { & $Body 2>&1 | ForEach-Object { $lines.Add("$_") } } catch { $ok = $false; $lines.Add($_.Exception.Message) }
  $output = $lines -join "`n"
  Add-Content -Path $Log -Value $output
  $detail = if ($output.Length -gt 1500) { $output.Substring($output.Length - 1500) } else { $output }
  [void]$Steps.Add([ordered]@{ name = $Name; ok = $ok; seconds = [int]((Get-Date) - $started).TotalSeconds; detail = $detail })
}

function Skip([string]$Name) { [void]$Steps.Add([ordered]@{ name = $Name; ok = $null; seconds = 0; detail = "skipped: set-up did not finish" }) }

function Get-Facts {
  $policies = (Get-ExecutionPolicy -List | ForEach-Object { "$($_.Scope)=$($_.ExecutionPolicy)" }) -join ", "
  # What a person typing `npx` in a new Windows PowerShell gets under this machine's execution policy. This process
  # runs with -ExecutionPolicy Bypass, which reaches children through PSExecutionPolicyPreference; drop it for this one.
  $inherited = $env:PSExecutionPolicyPreference
  Remove-Item Env:PSExecutionPolicyPreference -ErrorAction SilentlyContinue
  $npxPs1 = (& powershell.exe -NoProfile -Command "npx --version" 2>&1 | ForEach-Object { "$_" }) -join " "
  $env:PSExecutionPolicyPreference = $inherited
  return [ordered]@{
    os = (Get-CimInstance Win32_OperatingSystem).Caption
    build = [Environment]::OSVersion.Version.ToString()
    arch = $env:PROCESSOR_ARCHITECTURE
    node = (& node --version)
    npm = (& npm.cmd --version)
    git = (& git --version)
    powershell = $PSVersionTable.PSVersion.ToString()
    executionPolicy = $policies
    npxInWindowsPowerShell = $npxPs1.Trim()
    session = "$((Get-Process -Id $PID).SessionId) (0 is services, 1+ a signed-in desktop)"
    elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  }
}

$facts = Get-Facts

if ($RerailEnv.BundleUrl) {
  # Packages under test (core and every rule pack) from a registry on this VM, as npmjs will serve them after
  # launch; everything else still comes from npmjs.
  Step "registry" {
    $tarballs = @(Get-ChildItem "$Work\bundle\*.tgz" | ForEach-Object { $_.FullName })
    $State.Registry = Start-RerailRegistry -Root "$Out\registry" -Tarballs $tarballs
    Write-RerailUtf8 (Join-Path $env:USERPROFILE ".npmrc") "@neralabs:registry=$($State.Registry.Url)/`n"
    "registry $($State.Registry.Url) serves: $(($tarballs | ForEach-Object { Split-Path -Leaf $_ }) -join ' ')"
  }
} else {
  Step "registry" {
    if ($RerailEnv.NpmToken) { Write-RerailUtf8 (Join-Path $env:USERPROFILE ".npmrc") "//registry.npmjs.org/:_authToken=$($RerailEnv.NpmToken)`n" }
    "installing @neralabs/rerail@$($RerailEnv.NpmSpec) from npmjs"
  }
}

Step "project" {
  New-Item -ItemType Directory -Force "$Project\src", "$env:USERPROFILE\.claude", "$env:USERPROFILE\.codex", "$env:USERPROFILE\.cursor" | Out-Null
  Set-Location $Project
  Exec git @("init", "-q") | Out-Null
  Write-RerailUtf8 "$Project\package.json" '{ "name": "proj", "private": true, "dependencies": { "react": "^19.0.0" }, "devDependencies": { "typescript": "^5.0.0" } }'
  Write-RerailUtf8 "$Project\src\index.ts" "export const answer = 42;`n"
  Exec git @("add", "-A") | Out-Null
  Exec git @("-c", "user.name=rerail", "-c", "user.email=rerail@example.invalid", "commit", "-qm", "init") | Out-Null
  "project at $Project; coding-agent folders: .claude .codex .cursor"
}

Step "setup" {
  $spec = if ($RerailEnv.NpmSpec) { $RerailEnv.NpmSpec } else { "latest" }
  Invoke-RerailSetup -Spec $spec -Project $Project -Launcher $Launcher -LogDir $Out
  $State.SetupOk = $true
}

if ($State.SetupOk) {
  Step "status" {
    Assert-RerailManagedInstall -RerailHome $RerailHome
    $status = Exec $Launcher @("status")
    $status
    if ($status -notmatch "Engine: ok") { throw "the daemon has no working native engine" }
    $match = [regex]::Match($status, "running at http://127\.0\.0\.1:(\d+)/")
    if (-not $match.Success) { throw "no daemon URL in rerail status" }
    $State.DaemonPort = $match.Groups[1].Value
  }

  # A new terminal builds PATH from the machine and user values in the registry; set-up wrote the user one.
  Step "path-new-terminal" {
    $fresh = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [Environment]::GetEnvironmentVariable("Path", "User")
    $current = $env:Path
    try {
      $env:Path = $fresh
      $found = Get-Command rerail -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    } finally { $env:Path = $current }
    if ($null -eq $found -or $found.Source -ne $Launcher) { throw "a new terminal does not find $Launcher (found: $($found.Source))" }
    "a new terminal finds $($found.Source)"
  }

  Step "hooks" {
    Invoke-RerailSessionStartHooks -UserHome $env:USERPROFILE -Project $Project -WorkDir $Out -Prefix "env"
    if ((Get-Content -Raw "$env:USERPROFILE\.cursor\hooks.json") -notmatch "rerail") { throw "Cursor hooks were not registered" }
    Assert-RerailSessionsRecorded -Port $State.DaemonPort -SessionIds @("env-claude", "env-codex")
  }

  Step "daemon-restart" {
    $stop = Start-Process -PassThru -NoNewWindow -FilePath $Launcher -ArgumentList "daemon", "stop"
    if (-not $stop.WaitForExit(60000)) { throw "rerail daemon stop did not return within 60 s" }
    & $Launcher status *> $null
    if ($LASTEXITCODE -eq 0) { throw "status still reports a running daemon after stop" }
    Exec $Launcher @("daemon", "start")
    $status = Exec $Launcher @("status")
    if ($status -notmatch "Engine: ok") { throw "the restarted daemon has no working native engine:`n$status" }
    "restarted: Engine ok"
  }

  Step "uninstall" {
    Exec $Launcher @("install", "--remove")
    $leftovers = @(Get-RerailHookLeftovers -UserHome $env:USERPROFILE)
    if ($leftovers.Count -gt 0) { throw "Rerail hooks left in $($leftovers -join ', ')" }
    if ([Environment]::GetEnvironmentVariable("Path", "User") -match [regex]::Escape(".rerail\bin")) { throw "the user Path still holds .rerail\bin" }
    "Claude Code, Codex and Cursor unhooked, and the user Path entry removed"
  }
} else {
  foreach ($name in @("status", "path-new-terminal", "hooks", "daemon-restart", "uninstall")) { Skip $name }
}

if (Test-Path $Launcher) { & $Launcher daemon stop *> $null }
Stop-RerailRegistry $State.Registry

# The log first: the driver collects both once results.json appears.
$provisionLog = Join-Path $Work "provision.log"
Write-RerailUtf8 "$Out\all.log" ($(if (Test-Path $provisionLog) { Get-Content -Raw $provisionLog } else { "" }) + "`n" + (Get-Content -Raw $Log))
Send-Blob "$Out\all.log" $RerailEnv.LogUrl
$results = [ordered]@{ env = $RerailEnv.Id; provisioned = $true; facts = $facts; steps = @($Steps) }
Write-RerailUtf8 "$Out\results.json" ($results | ConvertTo-Json -Depth 6)
Send-Blob "$Out\results.json" $RerailEnv.ResultUrl
