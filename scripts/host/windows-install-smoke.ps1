# Release gate for native Windows: install a packed @neralabs/rerail tarball the way a user does, with
# `npx @neralabs/rerail setup`, from a throwaway local registry standing in for npmjs, in a sandbox profile, answering
# set-up's questions through its machine-readable lane. Then exercise the managed runtime: launcher, `current`
# junction, daemon and native engine, Claude Code's exec-form hook, the Codex hook as Codex runs it (PowerShell),
# update check, and the full uninstall. Uses `node` from PATH, so the runner's architecture decides
# which native engine is exercised. The environment matrix (scripts/host/azure/guest.ps1) runs the same steps from
# windows-host.ps1 on Windows 11 VMs.
#
# Only the @neralabs scope resolves through the local registry (Verdaccio), which serves the package and this
# checkout's rule packs, as npmjs serves them; every dependency still comes from npmjs, exactly as for a user who
# installs the published package.
param([Parameter(Mandatory)][string]$Tarball)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "windows-host.ps1")
$Tarball = (Resolve-Path $Tarball).Path
$base = Join-Path ([IO.Path]::GetTempPath()) ("rerail-windows-smoke-" + [guid]::NewGuid())
# A profile folder as a person's name gives it, accented and with a space: cmd.exe reads the batch launcher in the
# console's code page, so a path written out in UTF-8 would come back garbled, and PowerShell, which runs the Codex
# hook, splits a command at a space its quoting does not cover.
$sandbox = Join-Path $base ("Jos" + [char]0x00E9 + " Garc" + [char]0x00ED + "a")
$project = Join-Path $sandbox "proj"
$rerailHome = Join-Path $sandbox ".rerail"
$launcher = Join-Path $rerailHome "bin\rerail.cmd"

function Step($message) { Write-Host "==> $message" }

New-Item -ItemType Directory -Force $project, "$sandbox\.claude", "$sandbox\.codex", "$sandbox\.cursor", "$base\npm-cache", "$base\git-bash-home" | Out-Null
$registry = $null
try {
  Step "Packing this checkout's rule packs"
  # Set-up downloads the packs that fit the project; with none to download it offers only to try again.
  New-Item -ItemType Directory -Force "$base\packs" | Out-Null
  $node = Get-RerailNode
  $npmCli = Join-Path (Split-Path -Parent $node) "node_modules\npm\bin\npm-cli.js"
  foreach ($pack in Get-ChildItem -Directory (Join-Path $PSScriptRoot "..\..\packs")) {
    if (-not (Test-Path (Join-Path $pack.FullName "package.json"))) { continue }
    & $node $npmCli pack $pack.FullName --pack-destination "$base\packs" --ignore-scripts --silent | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "npm pack $($pack.Name) exited $LASTEXITCODE" }
  }
  $packTarballs = @(Get-ChildItem "$base\packs" -Filter *.tgz | ForEach-Object { $_.FullName })
  Write-Host "packed $($packTarballs.Count) rule packs"

  Step "Starting a local registry for the @neralabs scope"
  $registry = Start-RerailRegistry -Root "$base\registry" -Tarballs (@($Tarball) + $packTarballs)

  Step "Running npx @neralabs/rerail setup in a sandbox profile, answering its questions (stdout captured)"
  # os.homedir() reads USERPROFILE on Windows; the rest pins npm and Rerail state, including the one Rerail
  # database under RERAIL_HOME, inside the sandbox. The user config maps only the @neralabs scope to the
  # local registry, so Rerail's own npm calls (install, update check) resolve it the same way. BROWSER=none keeps
  # the dashboard link from opening a browser on the runner. HOME names another folder, as Git Bash's may: coding
  # agents read the profile folder, so set-up must connect them there.
  $env:USERPROFILE = $sandbox; $env:HOME = "$base\git-bash-home"; $env:RERAIL_HOME = $rerailHome
  $env:NPM_CONFIG_USERCONFIG = "$sandbox\.npmrc"; $env:NPM_CONFIG_CACHE = "$base\npm-cache"
  $env:RERAIL_STORAGE_ROOT = "$project\.rerail\sessions"
  $env:BROWSER = "none"
  Write-RerailUtf8 "$sandbox\.npmrc" "@neralabs:registry=$($registry.Url)/`n"
  $daemonPort = Get-RerailFreePort
  Invoke-RerailSetup -Spec "latest" -Project $project -Launcher $launcher -Options @("--port", "$daemonPort") | ForEach-Object { Write-Host $_ }

  Step "Checking the managed installation"
  Assert-RerailManagedInstall -RerailHome $rerailHome | ForEach-Object { Write-Host $_ }
  $status = (& $launcher status) -join "`n"
  Write-Host $status
  if ($status -notmatch [regex]::Escape("running at http://127.0.0.1:$daemonPort")) { throw "rerail status does not show the setup-started daemon" }
  if ($status -notmatch "Engine: ok") { throw "the daemon has no working native engine" }

  Step "Running rerail from Git Bash"
  # Git Bash runs a command without an extension or with `.exe`, never `rerail.cmd`; a new terminal gets the user Path
  # set-up wrote, which this one predates.
  $windowsPath = $env:PATH
  $env:PATH = "$rerailHome\bin;$windowsPath"
  $gitBash = (& (Join-Path $env:ProgramFiles "Git\bin\bash.exe") -c 'command -v rerail && rerail version') -join "`n"
  $gitBashExit = $LASTEXITCODE
  $env:PATH = $windowsPath
  Write-Host $gitBash
  if ($gitBashExit -ne 0) { throw "bare rerail did not run in Git Bash (exit $gitBashExit)" }

  Step "Running rerail.cmd in a console on code page 437"
  # The runner's console is on UTF-8 (65001); a person's is on their OEM code page, 437 or 850 in the West. A console of
  # its own (CreateNoWindow) keeps the chcp from reaching this one.
  $env:SMOKE_LAUNCHER = $launcher
  $legacy = New-Object Diagnostics.ProcessStartInfo("cmd.exe")
  $legacy.Arguments = '/d /s /c "chcp 437 >nul & call "%SMOKE_LAUNCHER%" version"'
  $legacy.UseShellExecute = $false; $legacy.CreateNoWindow = $true
  $legacy.RedirectStandardOutput = $true; $legacy.RedirectStandardError = $true
  $legacyRun = [Diagnostics.Process]::Start($legacy)
  $legacyOut = $legacyRun.StandardOutput.ReadToEndAsync(); $legacyErr = $legacyRun.StandardError.ReadToEndAsync()
  $legacyRun.WaitForExit()
  Write-Host ($legacyOut.Result + $legacyErr.Result)
  if ($legacyRun.ExitCode -ne 0) { throw "rerail.cmd failed on code page 437 (exit $($legacyRun.ExitCode))" }

  Step "A launcher whose recorded node is gone runs node.exe from PATH, never a repository's"
  # cmd tries the current folder before PATH for a bare command name, so a repository holding a node.bat would run it
  # on every `rerail`. A copy of the launcher, beside it so %~dp0 still finds `current`, records a node that is gone.
  $fallback = "$rerailHome\bin\rerail-fallback.cmd"
  $lines = (Get-Content $launcher) -replace '^set "RERAIL_NODE=.*"$', 'set "RERAIL_NODE=C:\gone\node.exe"'
  [IO.File]::WriteAllLines($fallback, [string[]]$lines)
  New-Item -ItemType Directory -Force "$base\hostile-repo" | Out-Null
  Write-RerailUtf8 "$base\hostile-repo\node.bat" "@echo the repository's node.bat ran`r`n"
  $env:SMOKE_FALLBACK = $fallback
  $hostile = New-Object Diagnostics.ProcessStartInfo("cmd.exe")
  $hostile.Arguments = '/d /s /c "call "%SMOKE_FALLBACK%" version"'
  $hostile.WorkingDirectory = "$base\hostile-repo"
  $hostile.UseShellExecute = $false; $hostile.CreateNoWindow = $true
  $hostile.RedirectStandardOutput = $true; $hostile.RedirectStandardError = $true
  $hostileRun = [Diagnostics.Process]::Start($hostile)
  $hostileOut = $hostileRun.StandardOutput.ReadToEndAsync(); $hostileErr = $hostileRun.StandardError.ReadToEndAsync()
  $hostileRun.WaitForExit()
  Remove-Item -Force $fallback
  Write-Host ($hostileOut.Result + $hostileErr.Result)
  if (($hostileOut.Result + $hostileErr.Result) -match "node\.bat ran") { throw "rerail.cmd ran node.bat from the current folder" }
  if ($hostileRun.ExitCode -ne 0) { throw "rerail.cmd did not fall back to node.exe on PATH (exit $($hostileRun.ExitCode))" }

  Step "Starting Rerail in the project"
  # Set-up's "start" answer turned Rerail on in the project; starting again says it is already on.
  Write-Host ((& $launcher start --repo $project) -join "`n")
  if ($LASTEXITCODE -ne 0) { throw "rerail start exited $LASTEXITCODE" }

  Step "Running the harness hooks as each harness spawns them"
  Invoke-RerailSessionStartHooks -UserHome $sandbox -Project $project -WorkDir $base -Prefix "windows-smoke" | ForEach-Object { Write-Host $_ }
  Assert-RerailSessionsRecorded -Port $daemonPort -SessionIds @("windows-smoke-claude", "windows-smoke-codex") | ForEach-Object { Write-Host $_ }

  Step "rerail update check"
  $update = (& $launcher update check) -join "`n"
  Write-Host $update
  if ($update -notmatch "Update available: no") { throw "update check did not report 'Update available: no'" }

  Step "rerail.cmd passes on the CLI's exit code"
  # The launcher ends its batch before node runs (so install --remove can delete it); a failing command must still fail.
  $ErrorActionPreference = "Continue"
  & $launcher install --no-such-option 2>$null | Out-Null
  $failed = $LASTEXITCODE
  $ErrorActionPreference = "Stop"
  if ($failed -eq 0) { throw "rerail.cmd exited 0 for a command that failed" }

  Step "rerail install --remove"
  # stderr too: cmd says "The batch file cannot be found." there when the launcher is deleted under it.
  $ErrorActionPreference = "Continue"
  $removal = (& $launcher install --remove 2>&1 | ForEach-Object { "$_" }) -join "`n"
  $removed = $LASTEXITCODE
  $ErrorActionPreference = "Stop"
  Write-Host $removal
  if ($removed -ne 0) { throw "install --remove exited $removed" }
  $leftovers = @(Get-RerailHookLeftovers -UserHome $sandbox)
  if ($leftovers.Count -gt 0) { throw "install --remove left Rerail hooks in $($leftovers -join ', ')" }
  if ($removal -match "batch file cannot be found") { throw "cmd lost the launcher install --remove deleted under it" }
  # Listed by name: Test-Path calls a junction whose target is gone absent.
  $left = @(Get-ChildItem -Force -Name $rerailHome | Where-Object { $_ -in @("current", "bin", "versions", "update-state.json", "packs") })
  if (Test-Path "$sandbox\.cache\rerail") { $left += "$sandbox\.cache\rerail" }
  if ($left.Count -gt 0) { throw "install --remove left behind: $($left -join ', ')" }
  Step "windows-install-smoke: PASS"
} finally {
  if (Test-Path $launcher) { & $launcher daemon stop | Out-Null }
  Stop-RerailRegistry $registry
  Remove-Item -Recurse -Force $base -ErrorAction SilentlyContinue
}
