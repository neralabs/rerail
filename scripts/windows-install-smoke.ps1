# Release gate for native Windows: install a packed @neralabs/rerail tarball the way a user does, with
# `npx @neralabs/rerail setup`, from a throwaway local registry standing in for npmjs, in a sandbox profile. Then
# exercise the managed runtime: launcher, `current` junction, daemon and native kernel, Claude Code's exec-form
# hook, the Codex hook as Codex runs it (PowerShell), update check, and harness removal. Uses `node` from PATH,
# so the runner's architecture decides which native kernel is exercised.
#
# Only the @neralabs scope resolves through the local registry (Verdaccio); every dependency still comes from
# npmjs, exactly as for a user who installs the published package.
param([Parameter(Mandatory)][string]$Tarball)

$ErrorActionPreference = "Stop"
$Tarball = (Resolve-Path $Tarball).Path
$node = (Get-Command node).Source
$npmBin = Join-Path (Split-Path -Parent $node) "node_modules\npm\bin"
$npmCli = Join-Path $npmBin "npm-cli.js"
$npxCli = Join-Path $npmBin "npx-cli.js"
$base = Join-Path ([IO.Path]::GetTempPath()) ("rerail-windows-smoke-" + [guid]::NewGuid())
$sandbox = Join-Path $base "home"
$project = Join-Path $sandbox "proj"
$launcher = Join-Path $sandbox ".rerail\bin\rerail.cmd"

function Step($message) { Write-Host "==> $message" }
function Invoke-Node([string[]]$arguments) {
  $output = & $node @arguments
  if ($LASTEXITCODE -ne 0) { throw "node $($arguments[0]) exited $LASTEXITCODE" }
  return $output
}
function Get-FreePort { Invoke-Node @("-e", "const s=require('net').createServer();s.listen(0,'127.0.0.1',()=>{process.stdout.write(String(s.address().port));s.close()})") }

New-Item -ItemType Directory -Force $project, "$sandbox\.claude", "$sandbox\.codex", "$sandbox\.cursor", "$base\storage", "$base\npm-cache" | Out-Null
$verdaccio = $null
try {
  Step "Starting a local registry for the @neralabs scope"
  $port = Get-FreePort
  $registry = "http://127.0.0.1:$port"
  # Readable by anyone, as npmjs serves a public package; publishing needs the throwaway account below.
  $password = Invoke-Node @("-e", "process.stdout.write(require('crypto').randomBytes(18).toString('hex'))")
  $sha = Invoke-Node @("-e", "process.stdout.write(require('crypto').createHash('sha1').update(process.argv[1]).digest('base64'))", $password)
  [IO.File]::WriteAllText("$base\htpasswd", "rerail-smoke:{SHA}$sha`n")
  $storage = "$base\storage" -replace '\\', '/'
  $authFile = "$base\htpasswd" -replace '\\', '/'
  [IO.File]::WriteAllText("$base\verdaccio.yaml", @"
storage: $storage
auth:
  htpasswd:
    file: $authFile
    max_users: -1
uplinks: {}
packages:
  "@neralabs/*":
    access: `$all
    publish: `$authenticated
max_body_size: 200mb
log: { type: stdout, level: warn }
"@)
  $encoded = Invoke-Node @("-p", "Buffer.from(process.argv[1]).toString('base64')", $password)
  [IO.File]::WriteAllText("$base\publisher.npmrc", "//127.0.0.1:$port/:username=rerail-smoke`n//127.0.0.1:$port/:_password=$encoded`n")
  # No output redirection: a redirected Start-Process inherits this shell's handles and would hold the caller's pipe.
  $verdaccio = Start-Process -PassThru -WindowStyle Hidden -FilePath $node -ArgumentList @(
    "`"$npmCli`"", "exec", "--yes", "--", "verdaccio@6", "--config", "`"$base\verdaccio.yaml`"", "--listen", "127.0.0.1:$port")
  $ready = $false
  for ($attempt = 0; $attempt -lt 120 -and -not $ready; $attempt++) {
    try { Invoke-WebRequest -UseBasicParsing "$registry/-/ping" | Out-Null; $ready = $true } catch { Start-Sleep 1 }
  }
  if (-not $ready) { throw "Verdaccio did not answer $registry/-/ping within 120s" }
  & $node $npmCli publish $Tarball --tag latest --registry $registry --userconfig "$base\publisher.npmrc" --loglevel=warn
  if ($LASTEXITCODE -ne 0) { throw "npm publish to the local registry failed" }

  Step "Running npx @neralabs/rerail setup in a sandbox profile (stdout captured)"
  # os.homedir() reads USERPROFILE on Windows; the rest pins npm and Rerail state, including the one Rerail
  # database under RERAIL_HOME, inside the sandbox. The user config maps only the @neralabs scope to the
  # local registry, so Rerail's own npm calls (install, update check) resolve it the same way.
  $env:USERPROFILE = $sandbox; $env:HOME = $sandbox; $env:RERAIL_HOME = "$sandbox\.rerail"
  $env:NPM_CONFIG_USERCONFIG = "$sandbox\.npmrc"; $env:NPM_CONFIG_CACHE = "$base\npm-cache"
  $env:RERAIL_STORAGE_ROOT = "$project\.rerail\sessions"
  [IO.File]::WriteAllText("$sandbox\.npmrc", "@neralabs:registry=$registry/`n")
  $daemonPort = Get-FreePort
  $start = New-Object Diagnostics.ProcessStartInfo($node)
  $start.Arguments = "`"$npxCli`" --yes @neralabs/rerail@latest setup --project `"$project`" --port $daemonPort"
  $start.WorkingDirectory = $project
  $start.UseShellExecute = $false; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
  $setup = [Diagnostics.Process]::Start($start)
  $stdout = $setup.StandardOutput.ReadToEndAsync(); $stderr = $setup.StandardError.ReadToEndAsync()
  $setup.WaitForExit()
  # The daemon must not inherit setup's output pipe, or every captured `rerail setup` hangs until `rerail stop`.
  if (-not $stdout.Wait(15000)) { throw "setup exited but its stdout stayed open: the daemon holds the caller's pipe" }
  Write-Host ($stdout.Result + $stderr.Result)
  if ($setup.ExitCode -ne 0) { throw "npx @neralabs/rerail setup exited $($setup.ExitCode)" }

  Step "Checking the managed installation"
  if (-not (Test-Path $launcher)) { throw "setup did not write $launcher" }
  if ((Get-Item "$sandbox\.rerail\current").LinkType -ne "Junction") { throw "$sandbox\.rerail\current is not a junction" }
  $status = (& $launcher status) -join "`n"
  Write-Host $status
  if ($status -notmatch [regex]::Escape("running at http://127.0.0.1:$daemonPort")) { throw "rerail status does not show the setup-started daemon" }
  if ($status -notmatch "Native kernel: ok") { throw "the daemon has no working native kernel" }

  Step "Running the harness hooks as each harness spawns them"
  $payload = @{ session_id = "windows-smoke"; cwd = $project; hook_event_name = "SessionStart"; source = "startup" } | ConvertTo-Json -Compress
  [IO.File]::WriteAllText("$base\payload.json", $payload)
  $claude = (Get-Content -Raw "$sandbox\.claude\settings.json" | ConvertFrom-Json).hooks.SessionStart[0].hooks[0]
  if ($null -eq $claude.args) { throw "the Claude Code hook is not in exec form: $($claude.command)" }
  # Claude Code spawns exec-form hooks directly (no shell); Codex passes its command to PowerShell.
  $probe = "const {spawnSync}=require('child_process'),fs=require('fs');const [kind,a,b]=process.argv.slice(1);const input=fs.readFileSync(process.env.SMOKE_PAYLOAD);" +
    "const r=kind==='exec'?spawnSync(a,JSON.parse(fs.readFileSync(b,'utf8')),{input,encoding:'utf8',cwd:process.env.SMOKE_PROJECT}):spawnSync('powershell.exe',['-NoProfile','-Command',fs.readFileSync(a,'utf8')],{input,encoding:'utf8',cwd:process.env.SMOKE_PROJECT});" +
    "process.stdout.write('exit='+r.status+' '+(r.stdout+r.stderr).trim());process.exitCode=r.status"
  $env:SMOKE_PAYLOAD = "$base\payload.json"; $env:SMOKE_PROJECT = $project
  [IO.File]::WriteAllText("$base\claude-args.json", (ConvertTo-Json -Compress @($claude.args)))
  Write-Host ("Claude Code exec-form hook: " + (Invoke-Node @("-e", "$probe", "exec", $claude.command, "$base\claude-args.json")))
  [IO.File]::WriteAllText("$base\codex-command.txt", (Get-Content -Raw "$sandbox\.codex\hooks.json" | ConvertFrom-Json).hooks.SessionStart[0].hooks[0].command)
  Write-Host ("Codex hook through PowerShell: " + (Invoke-Node @("-e", "$probe", "powershell", "$base\codex-command.txt")))

  Step "rerail update check"
  $update = (& $launcher update check) -join "`n"
  Write-Host $update
  if ($update -notmatch "Update available: no") { throw "update check did not report 'Update available: no'" }

  Step "rerail install --remove"
  & $launcher install --remove
  if ($LASTEXITCODE -ne 0) { throw "install --remove exited $LASTEXITCODE" }
  if ((Get-Content -Raw "$sandbox\.claude\settings.json") -match "claude-hook") { throw "install --remove left Claude Code hooks behind" }
  Step "windows-install-smoke: PASS"
} finally {
  if (Test-Path $launcher) { & $launcher stop | Out-Null }
  if ($null -ne $verdaccio) {
    # npm exec wraps Verdaccio in a child; stop the whole tree.
    & taskkill.exe /T /F /PID $verdaccio.Id 2>&1 | Out-Null
  }
  Remove-Item -Recurse -Force $base -ErrorAction SilentlyContinue
}
