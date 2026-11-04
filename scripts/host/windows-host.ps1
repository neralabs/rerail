# What the two Windows install checks share, so the release gate and the environment matrix cannot drift apart:
# scripts/host/windows-install-smoke.ps1 dot-sources this file, and scripts/host/azure/matrix.mjs puts it ahead of
# scripts/host/azure/guest.ps1 in the script it sends to a Windows VM. Windows PowerShell 5.1 and PowerShell 7.
#
# Each function keeps $ErrorActionPreference at Continue for itself (5.1 turns a native command's stderr into a
# terminating error under Stop) and throws on failure. Functions that report progress write it as output lines, as
# they go, so a caller that collects output keeps every line printed before a failure.

function Write-RerailUtf8([string]$File, [string]$Text) {
  [IO.File]::WriteAllText($File, $Text, (New-Object Text.UTF8Encoding $false))
}

function Get-RerailNode { (Get-Command node -CommandType Application | Select-Object -First 1).Source }

function Get-RerailFreePort {
  $port = & (Get-RerailNode) -e "const s=require('net').createServer();s.listen(0,'127.0.0.1',()=>{process.stdout.write(String(s.address().port));s.close()})"
  if ($LASTEXITCODE -ne 0) { throw "could not find a free port" }
  return [int]$port
}

# One command-line argument, quoted the way the C runtime (and so node and cmd's batch arguments) reads it back.
function ConvertTo-RerailArgument([string]$Value) {
  if ($Value -ne "" -and $Value -notmatch '[\s"]') { return $Value }
  return '"' + (($Value -replace '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
}

# Runs a program with stdout and stderr read through pipes. Its stdout must close within 15 s of its exit: a daemon
# that inherited the pipe would hold every captured `rerail setup` open (what a coding agent running set-up sees).
function Invoke-RerailCaptured([string]$File, [string[]]$Arguments, [string]$WorkingDirectory, [int]$TimeoutMinutes = 15) {
  $start = New-Object Diagnostics.ProcessStartInfo($File)
  $start.Arguments = (@($Arguments) | ForEach-Object { ConvertTo-RerailArgument $_ }) -join " "
  $start.WorkingDirectory = $WorkingDirectory
  $start.UseShellExecute = $false
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  $process = [Diagnostics.Process]::Start($start)
  $stdout = $process.StandardOutput.ReadToEndAsync()
  $stderr = $process.StandardError.ReadToEndAsync()
  if (-not $process.WaitForExit($TimeoutMinutes * 60000)) {
    & taskkill.exe /T /F /PID $process.Id *> $null
    throw "$File $($start.Arguments) did not finish within $TimeoutMinutes minutes"
  }
  if (-not $stdout.Wait(15000)) { throw "$File exited but its stdout stayed open: the daemon holds the caller's pipe" }
  [void]$stderr.Wait(15000)
  return @{ Exit = $process.ExitCode; Out = $stdout.Result; Err = $stderr.Result }
}

# A registry on 127.0.0.1 that serves the given tarballs under the @neralabs scope, readable by anyone as npmjs serves
# a public package. Map the scope to `.Url` in the user's .npmrc; every other package still comes from npmjs.
function Start-RerailRegistry([string]$Root, [string[]]$Tarballs) {
  $ErrorActionPreference = "Continue"
  $node = Get-RerailNode
  $npmCli = Join-Path (Split-Path -Parent $node) "node_modules\npm\bin\npm-cli.js"
  New-Item -ItemType Directory -Force $Root | Out-Null
  $port = Get-RerailFreePort
  $url = "http://127.0.0.1:$port"
  $storage = "$Root\storage" -replace '\\', '/'
  $htpasswd = "$Root\htpasswd" -replace '\\', '/'
  # One account, registered below, may publish.
  Write-RerailUtf8 "$Root\config.yaml" @"
storage: $storage
auth:
  htpasswd:
    file: $htpasswd
    max_users: 1
uplinks: {}
packages:
  "@neralabs/*":
    access: `$all
    publish: `$authenticated
max_body_size: 200mb
log: { type: stdout, level: warn }
"@
  # No output redirection: a redirected Start-Process inherits this shell's handles and would hold the caller's pipe.
  $process = Start-Process -PassThru -WindowStyle Hidden -FilePath $node -ArgumentList @(
    "`"$npmCli`"", "exec", "--yes", "--", "verdaccio@6", "--config", "`"$Root\config.yaml`"", "--listen", "127.0.0.1:$port")
  $ready = $false
  for ($attempt = 0; $attempt -lt 180 -and -not $ready; $attempt++) {
    try { Invoke-WebRequest -UseBasicParsing -ErrorAction Stop "$url/-/ping" | Out-Null; $ready = $true } catch { Start-Sleep 1 }
  }
  if (-not $ready) {
    & taskkill.exe /T /F /PID $process.Id *> $null
    throw "the local registry did not answer $url/-/ping within 180 s"
  }
  # A failed registration or publish stops the registry before it throws: the caller has no process to stop yet, and
  # a Verdaccio left running keeps the runner's work directory locked.
  try {
    $body = @{ name = "publisher"; password = [guid]::NewGuid().ToString("N") } | ConvertTo-Json -Compress
    $token = (Invoke-RestMethod -UseBasicParsing -ErrorAction Stop -Method Put -ContentType "application/json" -Body $body "$url/-/user/org.couchdb.user:publisher").token
    Write-RerailUtf8 "$Root\publisher.npmrc" "//127.0.0.1:${port}/:_authToken=$token`n"
    foreach ($tarball in $Tarballs) {
      $published = (& $node $npmCli publish $tarball --tag latest --registry $url --userconfig "$Root\publisher.npmrc" --loglevel=warn 2>&1) -join "`n"
      if ($LASTEXITCODE -ne 0) { throw "publishing $tarball to $url failed:`n$published" }
    }
  } catch {
    & taskkill.exe /T /F /PID $process.Id *> $null
    throw
  }
  return @{ Url = $url; Process = $process }
}

function Stop-RerailRegistry($Registry) {
  # npm exec wraps Verdaccio in a child; stop the whole tree.
  if ($null -ne $Registry -and $null -ne $Registry.Process) { & taskkill.exe /T /F /PID $Registry.Process.Id *> $null }
}

# Set-up's machine-readable lane to the end, answering as a person accepting the defaults would (allow sessions, read
# this repository, start with the packs shown): `npx @neralabs/rerail@<Spec> setup --json`, which installs, then for
# each answer the launcher command set-up printed for it. `Options` go on the first command (`--port 4799`); set-up
# carries them into the commands it prints. Writes `step <name>: <screen>` for each step, and each step's JSON to
# `LogDir` when given; throws unless set-up ends with Rerail on and every command it ran exited 0 (the lane exits 1
# only on its `error` step, 0 on a question).
function Invoke-RerailSetup([string]$Spec, [string]$Project, [string]$Launcher, [string[]]$Options = @(), [string]$LogDir = "") {
  $ErrorActionPreference = "Continue"
  $node = Get-RerailNode
  $npxCli = Join-Path (Split-Path -Parent $node) "node_modules\npm\bin\npx-cli.js"
  $answers = @{ "sessions" = "allow"; "read-repo" = "yes"; "confirm-packs" = "start" }
  $run = Invoke-RerailCaptured $node (@($npxCli, "--yes", "@neralabs/rerail@$Spec", "setup") + $Options + @("--json", "--repo", $Project)) $Project
  for ($i = 1; $i -le 6; $i++) {
    if ($LogDir) { Write-RerailUtf8 "$LogDir\setup-$i.json" $run.Out }
    # Windows PowerShell's ConvertFrom-Json returns nothing for empty input rather than failing.
    try { $step = $run.Out | ConvertFrom-Json } catch { $step = $null }
    if ($null -eq $step) { throw "set-up printed no JSON step (exit $($run.Exit)):`n$($run.Out)`n$($run.Err)" }
    "step $($step.step): $($step.screen)"
    if ($run.Exit -ne 0) { throw "set-up exited $($run.Exit) at step $($step.step):`n$($run.Err)" }
    if ($step.step -eq "done") {
      if ($step.screen -notmatch "Rerail is on") { throw "set-up finished without turning Rerail on" }
      return
    }
    if ($null -eq $step.question) { throw "set-up stopped at $($step.step) (exit $($run.Exit)):`n$($run.Err)" }
    $want = $answers[$step.question.id]
    $option = $step.question.options | Where-Object { $_.answer -eq $want } | Select-Object -First 1
    if ($null -eq $option) { throw "no '$want' answer for question $($step.question.id)" }
    $run = Invoke-RerailCaptured $Launcher @($option.next.args) $Project
  }
  throw "set-up did not finish in 6 answers"
}

# The launcher and the `current` junction a managed install writes under the Rerail home.
function Assert-RerailManagedInstall([string]$RerailHome) {
  if (-not (Test-Path "$RerailHome\bin\rerail.cmd")) { throw "set-up did not write $RerailHome\bin\rerail.cmd" }
  if ((Get-Item "$RerailHome\current").LinkType -ne "Junction") { throw "$RerailHome\current is not a junction" }
  "launcher and current junction in place under $RerailHome"
}

# Each agent's registered SessionStart command run the way that agent runs it: Claude Code spawns its exec-form entry
# directly (no shell), Codex passes its command to Windows PowerShell. The session ids are `<Prefix>-claude` and
# `<Prefix>-codex`; ask the daemon for them afterwards (Assert-RerailSessionsRecorded), since a hook where Rerail is
# off exits 0 too.
function Invoke-RerailSessionStartHooks([string]$UserHome, [string]$Project, [string]$WorkDir, [string]$Prefix) {
  $ErrorActionPreference = "Continue"
  $node = Get-RerailNode
  $claude = (Get-Content -Raw "$UserHome\.claude\settings.json" | ConvertFrom-Json).hooks.SessionStart[0].hooks[0]
  if ($null -eq $claude.args) { throw "the Claude Code hook is not in exec form: $($claude.command)" }
  Write-RerailUtf8 "$WorkDir\claude-args.json" (ConvertTo-Json -Compress @($claude.args))
  Write-RerailUtf8 "$WorkDir\codex-command.txt" (Get-Content -Raw "$UserHome\.codex\hooks.json" | ConvertFrom-Json).hooks.SessionStart[0].hooks[0].command
  $probe = "const {spawnSync}=require('child_process'),fs=require('fs');const [kind,a,b,payload,cwd]=process.argv.slice(1);const input=fs.readFileSync(payload);" +
    "const r=kind==='exec'?spawnSync(a,JSON.parse(fs.readFileSync(b,'utf8')),{input,encoding:'utf8',cwd}):spawnSync('powershell.exe',['-NoProfile','-Command',fs.readFileSync(a,'utf8')],{input,encoding:'utf8',cwd});" +
    "process.stdout.write('exit='+r.status+' '+(r.stdout+r.stderr).trim().slice(0,300));process.exitCode=r.status"
  foreach ($agent in @("claude", "codex")) {
    $payload = "$WorkDir\payload-$agent.json"
    Write-RerailUtf8 $payload (@{ session_id = "$Prefix-$agent"; cwd = $Project; hook_event_name = "SessionStart"; source = "startup" } | ConvertTo-Json -Compress)
    $probeArguments = if ($agent -eq "claude") { @("exec", $claude.command, "$WorkDir\claude-args.json") } else { @("powershell", "$WorkDir\codex-command.txt", "-") }
    $result = (& $node -e $probe @probeArguments $payload $Project 2>&1) -join " "
    "${agent}: $result"
    if ($LASTEXITCODE -ne 0) { throw "the $agent SessionStart hook exited $LASTEXITCODE" }
  }
}

# The daemon answers only a request that carries the access capability its `daemon.json` records, in the Rerail home as
# Rerail resolves it: `RERAIL_HOME`, else `.rerail` in the user profile.
function Get-RerailAccessHeaders {
  $rerailHome = if ($env:RERAIL_HOME) { $env:RERAIL_HOME } else { Join-Path $env:USERPROFILE ".rerail" }
  $token = (Get-Content -Raw (Join-Path $rerailHome "daemon.json") | ConvertFrom-Json).accessToken
  if (-not $token) { throw "$rerailHome\daemon.json records no access capability" }
  return @{ "x-rerail-access" = $token }
}

function Assert-RerailSessionsRecorded([int]$Port, [string[]]$SessionIds) {
  $sessions = Invoke-RestMethod -UseBasicParsing -ErrorAction Stop -Headers (Get-RerailAccessHeaders) "http://127.0.0.1:$Port/api/sessions"
  $recorded = @($sessions.sessions | ForEach-Object { $_.sessionId })
  "daemon recorded: $($recorded -join ', ')"
  $missing = @($SessionIds | Where-Object { $recorded -notcontains $_ })
  if ($missing.Count -gt 0) { throw "the daemon did not record $($missing -join ', ')" }
}

# The coding-agent config files that still point into a Rerail home (`\.rerail\`) after `rerail install --remove`.
function Get-RerailHookLeftovers([string]$UserHome) {
  foreach ($file in @(".claude\settings.json", ".codex\hooks.json", ".cursor\hooks.json")) {
    $path = Join-Path $UserHome $file
    if ((Test-Path $path) -and ((Get-Content -Raw $path) -match '[\\/]\.rerail[\\/]')) { $path }
  }
}
