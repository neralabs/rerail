# Runs on a throwaway Azure Windows VM for scripts/host/azure/matrix.mjs, which prepends `$RerailEnv` (a hashtable
# of settings) and sends this file through Azure Run Command. Windows PowerShell 5.1: nothing here needs PowerShell 7.
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
function Write-Utf8([string]$File, [string]$Text) { [IO.File]::WriteAllText($File, $Text, (New-Object Text.UTF8Encoding $false)) }

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
    $sums = (Invoke-WebRequest -UseBasicParsing "https://nodejs.org/dist/latest-v$($RerailEnv.Node).x/SHASUMS256.txt").Content
    $msi = ([regex]::Match($sums, "node-v[\d.]+-$arch\.msi")).Value
    if (-not $msi) { throw "no Node $($RerailEnv.Node).x installer for $arch" }
    $version = ([regex]::Match($msi, "v[\d.]+")).Value
    Invoke-WebRequest -UseBasicParsing "https://nodejs.org/dist/$version/$msi" -OutFile "$Work\$msi"
    $install = Start-Process msiexec.exe -ArgumentList "/i `"$Work\$msi`" /qn /norestart" -Wait -PassThru
    if ($install.ExitCode -ne 0) { throw "Node installer exited $($install.ExitCode)" }
    # Git for Windows, machine-wide in its default folder, as its installer puts it.
    $release = Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/git-for-windows/git/releases/latest"
    $pattern = if ($arch -eq "arm64") { '^Git-[\d.]+-arm64\.exe$' } else { '^Git-[\d.]+-64-bit\.exe$' }
    $asset = $release.assets | Where-Object { $_.name -match $pattern } | Select-Object -First 1
    if ($null -eq $asset) { throw "no Git for Windows installer for $arch in $($release.tag_name)" }
    Invoke-WebRequest -UseBasicParsing $asset.browser_download_url -OutFile "$Work\$($asset.name)"
    $install = Start-Process "$Work\$($asset.name)" -ArgumentList "/VERYSILENT /NORESTART /NOCANCEL /SP- /SUPPRESSMSGBOXES" -Wait -PassThru
    if ($install.ExitCode -ne 0) { throw "Git installer exited $($install.ExitCode)" }
    if ($RerailEnv.BundleUrl) {
      Invoke-WebRequest -UseBasicParsing $RerailEnv.BundleUrl -OutFile "$Work\bundle.tar"
      New-Item -ItemType Directory -Force "$Work\bundle" | Out-Null
      & tar.exe -xf "$Work\bundle.tar" -C "$Work\bundle"
      if ($LASTEXITCODE -ne 0) { throw "could not unpack the bundle" }
    }
    Stop-Transcript | Out-Null
  } catch {
    Write-Host "provisioning failed: $_"
    Stop-Transcript | Out-Null
    Write-Utf8 "$Work\results.json" (@{ env = $RerailEnv.Id; provisioned = $false; facts = @{}; steps = @() } | ConvertTo-Json -Compress)
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
$Launcher = Join-Path $env:USERPROFILE ".rerail\bin\rerail.cmd"
New-Item -ItemType Directory -Force $Out | Out-Null
Write-Utf8 $Log ""
$env:BROWSER = "none"; $env:NPM_CONFIG_UPDATE_NOTIFIER = "false"; $env:NO_COLOR = "1"
$Steps = New-Object Collections.ArrayList
$State = @{ SetupOk = $false; DaemonPort = $null; Registry = $null; Verdaccio = $null }

# Runs a native command, returning its combined output as one string and failing on a non-zero exit.
function Exec([string]$File, [string[]]$Arguments) {
  $output = (& $File @Arguments 2>&1 | ForEach-Object { "$_" }) -join "`n"
  if ($LASTEXITCODE -ne 0) { throw "$File $($Arguments -join ' ') exited ${LASTEXITCODE}:`n$output" }
  return $output
}

function Step([string]$Name, [scriptblock]$Body) {
  Add-Content -Path $Log -Value "`n==== $Name"
  $started = Get-Date
  $ok = $true
  try { $output = (& $Body 2>&1 | ForEach-Object { "$_" }) -join "`n" } catch { $ok = $false; $output = "$output`n$($_.Exception.Message)" }
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
    $node = (Get-Command node).Source
    $npmCli = Join-Path (Split-Path -Parent $node) "node_modules\npm\bin\npm-cli.js"
    $port = Exec $node @("-e", "const s=require('net').createServer();s.listen(0,'127.0.0.1',()=>{process.stdout.write(String(s.address().port));s.close()})")
    $State.Registry = "http://127.0.0.1:$port"
    $storage = "$Out\verdaccio\storage" -replace '\\', '/'
    $htpasswd = "$Out\verdaccio\htpasswd" -replace '\\', '/'
    New-Item -ItemType Directory -Force "$Out\verdaccio" | Out-Null
    Write-Utf8 "$Out\verdaccio\config.yaml" @"
storage: $storage
auth:
  htpasswd:
    file: $htpasswd
    max_users: 1
uplinks:
  npmjs:
    url: https://registry.npmjs.org/
packages:
  "@neralabs/*":
    access: `$all
    publish: `$authenticated
  "**":
    access: `$all
    proxy: npmjs
max_body_size: 200mb
log: { type: stdout, level: warn }
"@
    # No output redirection: a redirected Start-Process would hold this shell's handles.
    $State.Verdaccio = Start-Process -PassThru -WindowStyle Hidden -FilePath $node -ArgumentList @(
      "`"$npmCli`"", "exec", "--yes", "--", "verdaccio@6", "--config", "`"$Out\verdaccio\config.yaml`"", "--listen", "127.0.0.1:$port")
    $ready = $false
    for ($i = 0; $i -lt 180 -and -not $ready; $i++) {
      try { Invoke-WebRequest -UseBasicParsing "$($State.Registry)/-/ping" | Out-Null; $ready = $true } catch { Start-Sleep 1 }
    }
    if (-not $ready) { throw "the local registry did not answer within 180 s" }
    $password = [guid]::NewGuid().ToString("N")
    $body = @{ name = "publisher"; password = $password } | ConvertTo-Json -Compress
    $token = (Invoke-RestMethod -UseBasicParsing -Method Put -ContentType "application/json" -Body $body "$($State.Registry)/-/user/org.couchdb.user:publisher").token
    Write-Utf8 "$Out\publisher.npmrc" "//127.0.0.1:$port/:_authToken=$token`n"
    foreach ($tarball in Get-ChildItem "$Work\bundle\*.tgz") {
      Exec "npm.cmd" @("publish", $tarball.FullName, "--tag", "latest", "--registry", $State.Registry, "--userconfig", "$Out\publisher.npmrc", "--loglevel=warn") | Out-Null
    }
    Write-Utf8 (Join-Path $env:USERPROFILE ".npmrc") "@neralabs:registry=$($State.Registry)/`n"
    "registry $($State.Registry) serves: $((Get-ChildItem "$Work\bundle\*.tgz" | ForEach-Object { $_.Name }) -join ' ')"
  }
} else {
  Step "registry" {
    if ($RerailEnv.NpmToken) { Write-Utf8 (Join-Path $env:USERPROFILE ".npmrc") "//registry.npmjs.org/:_authToken=$($RerailEnv.NpmToken)`n" }
    "installing @neralabs/rerail@$($RerailEnv.NpmSpec) from npmjs"
  }
}

Step "project" {
  New-Item -ItemType Directory -Force "$Project\src", "$env:USERPROFILE\.claude", "$env:USERPROFILE\.codex", "$env:USERPROFILE\.cursor" | Out-Null
  Set-Location $Project
  Exec git @("init", "-q") | Out-Null
  Write-Utf8 "$Project\package.json" '{ "name": "proj", "private": true, "dependencies": { "react": "^19.0.0" }, "devDependencies": { "typescript": "^5.0.0" } }'
  Write-Utf8 "$Project\src\index.ts" "export const answer = 42;`n"
  Exec git @("add", "-A") | Out-Null
  Exec git @("-c", "user.name=rerail", "-c", "user.email=rerail@example.invalid", "commit", "-qm", "init") | Out-Null
  "project at $Project; coding-agent folders: .claude .codex .cursor"
}

# Set-up's machine-readable lane: each step is a JSON object whose options carry the exact launcher arguments for
# each answer. Answer as a person accepting the defaults would: allow sessions, read this repo, start with the packs
# shown. The first run is npx (it installs); later answers go to the installed launcher.
Step "setup" {
  Set-Location $Project
  $spec = if ($RerailEnv.NpmSpec) { $RerailEnv.NpmSpec } else { "latest" }
  $json = (& npx.cmd --yes "@neralabs/rerail@$spec" setup --json --repo $Project 2> "$Out\setup-stderr.txt") -join "`n"
  $answers = @{ "sessions" = "allow"; "read-repo" = "yes"; "confirm-packs" = "start" }
  for ($i = 1; $i -le 6; $i++) {
    Write-Utf8 "$Out\setup-$i.json" $json
    # Windows PowerShell's ConvertFrom-Json returns nothing for empty input rather than failing.
    try { $step = $json | ConvertFrom-Json } catch { $step = $null }
    if ($null -eq $step) { throw "set-up printed no JSON step: $json`n$(Get-Content -Raw "$Out\setup-stderr.txt")" }
    "step $($step.step): $($step.screen)"
    if ($step.step -eq "done") {
      if ($step.screen -notmatch "Rerail is on") { throw "set-up finished without turning Rerail on" }
      $State.SetupOk = $true
      return
    }
    if ($null -eq $step.question) { throw "set-up stopped at $($step.step)" }
    $want = $answers[$step.question.id]
    $option = $step.question.options | Where-Object { $_.answer -eq $want } | Select-Object -First 1
    if ($null -eq $option) { throw "no '$want' answer for question $($step.question.id)" }
    $json = (& $Launcher @($option.next.args) 2> "$Out\setup-stderr.txt") -join "`n"
  }
  throw "set-up did not finish in 6 answers"
}

if ($State.SetupOk) {
  Step "status" {
    $status = Exec $Launcher @("status")
    $status
    if ((Get-Item "$env:USERPROFILE\.rerail\current").LinkType -ne "Junction") { throw "~\.rerail\current is not a junction" }
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

  # Each agent's registered SessionStart command run the way that agent runs it (Claude Code: exec form; Codex:
  # through PowerShell), then the daemon is asked what it recorded: an exit code alone proves nothing.
  Step "hooks" {
    $payloads = @{}
    foreach ($agent in @("claude", "codex")) {
      $payloads[$agent] = "$Out\payload-$agent.json"
      Write-Utf8 $payloads[$agent] (@{ session_id = "env-$agent"; cwd = $Project; hook_event_name = "SessionStart"; source = "startup" } | ConvertTo-Json -Compress)
    }
    $claude = (Get-Content -Raw "$env:USERPROFILE\.claude\settings.json" | ConvertFrom-Json).hooks.SessionStart[0].hooks[0]
    if ($null -eq $claude.args) { throw "the Claude Code hook is not in exec form: $($claude.command)" }
    Write-Utf8 "$Out\claude-args.json" (ConvertTo-Json -Compress @($claude.args))
    Write-Utf8 "$Out\codex-command.txt" (Get-Content -Raw "$env:USERPROFILE\.codex\hooks.json" | ConvertFrom-Json).hooks.SessionStart[0].hooks[0].command
    $probe = "const {spawnSync}=require('child_process'),fs=require('fs');const [kind,a,b,payload,cwd]=process.argv.slice(1);const input=fs.readFileSync(payload);" +
      "const r=kind==='exec'?spawnSync(a,JSON.parse(fs.readFileSync(b,'utf8')),{input,encoding:'utf8',cwd}):spawnSync('powershell.exe',['-NoProfile','-Command',fs.readFileSync(a,'utf8')],{input,encoding:'utf8',cwd});" +
      "process.stdout.write('exit='+r.status+' '+(r.stdout+r.stderr).trim().slice(0,300));process.exitCode=r.status"
    "claude: " + (Exec node @("-e", $probe, "exec", $claude.command, "$Out\claude-args.json", $payloads.claude, $Project))
    "codex: " + (Exec node @("-e", $probe, "powershell", "$Out\codex-command.txt", "-", $payloads.codex, $Project))
    if ((Get-Content -Raw "$env:USERPROFILE\.cursor\hooks.json") -notmatch "rerail") { throw "Cursor hooks were not registered" }
    $recorded = @((Invoke-RestMethod -UseBasicParsing "http://127.0.0.1:$($State.DaemonPort)/api/sessions").sessions | ForEach-Object { $_.sessionId })
    "daemon recorded: $($recorded -join ', ')"
    foreach ($agent in @("claude", "codex")) {
      if ($recorded -notcontains "env-$agent") { throw "the $agent hook did not record its session" }
    }
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
    if ((Get-Content -Raw "$env:USERPROFILE\.claude\settings.json") -match "claude-hook") { throw "Claude Code hooks left behind" }
    if ([Environment]::GetEnvironmentVariable("Path", "User") -match [regex]::Escape(".rerail\bin")) { throw "the user Path still holds .rerail\bin" }
    "agents unhooked and the user Path entry removed"
  }
} else {
  foreach ($name in @("status", "path-new-terminal", "hooks", "daemon-restart", "uninstall")) { Skip $name }
}

if (Test-Path $Launcher) { & $Launcher daemon stop *> $null }
if ($null -ne $State.Verdaccio) { & taskkill.exe /T /F /PID $State.Verdaccio.Id *> $null }

# The log first: the driver collects both once results.json appears.
$provisionLog = Join-Path $Work "provision.log"
Write-Utf8 "$Out\all.log" ($(if (Test-Path $provisionLog) { Get-Content -Raw $provisionLog } else { "" }) + "`n" + (Get-Content -Raw $Log))
Send-Blob "$Out\all.log" $RerailEnv.LogUrl
$results = [ordered]@{ env = $RerailEnv.Id; provisioned = $true; facts = $facts; steps = @($Steps) }
Write-Utf8 "$Out\results.json" ($results | ConvertTo-Json -Depth 6)
Send-Blob "$Out\results.json" $RerailEnv.ResultUrl
