# A Windows host's end of the farm's dashboard (scripts/build/farm/dashboard): `sampler`'s
# counterpart in Windows' own PowerShell (5.1, which every Windows has), run over the machine's
# Windows ssh (`windows_ssh`), so a Windows machine's card is Windows' own view and stays live while
# its Linux (WSL) is stopped. It prints the same JSON line as `sampler` every PERIOD seconds.
#
# It runs in sshd's own PowerShell (the dashboard sends it as the command), which keeps its input:
# no pings (the card shows the farm's probe's round trip), and no end of input when the dashboard
# goes; nor does a write fail then, as the connection is shared. So it ends itself after LIFETIME
# with a `{"bye": true}` line, and the dashboard starts the next at once: one a dashboard left
# behind lives LIFETIME at most.
#
# Cheap by construction: the CPU and memory from two performance counters made once; the
# processes (for the farm's share: the trees of its Windows jobs, the batch files under the farm's
# directory, and WSL's VM) from one process list a sample, with a process's parent and command line
# asked of CIM once, when it is first seen.
$ErrorActionPreference = "SilentlyContinue"
# Below the farm's jobs, not Idle: under a Windows test run's load an Idle process got no CPU at all,
# even with half the machine free, and stopped in its setup with no sample sent.
(Get-Process -Id $PID).PriorityClass = "BelowNormal"
$PERIOD = 3.0
$LIFETIME = 600
$out = [Console]::Out
function Emit($obj) { try { $out.WriteLine(($obj | ConvertTo-Json -Compress -Depth 5)); $out.Flush() } catch { exit 0 } }

$os = Get-CimInstance Win32_OperatingSystem
$cs = Get-CimInstance Win32_ComputerSystem
$cpuName = ((Get-CimInstance Win32_Processor | Select-Object -First 1).Name).Trim()
$installed = (Get-CimInstance Win32_PhysicalMemory | Measure-Object Capacity -Sum).Sum
$arch = @{AMD64 = "x86_64"; ARM64 = "aarch64"; x86 = "i686"}[$env:PROCESSOR_ARCHITECTURE]
$virt = $null
if ($cs.Manufacturer -match "Parallels") { $virt = "parallels" } elseif ($cs.Model -match "Virtual") { $virt = "vm" }
$system = @{family = "windows"; name = ($os.Caption -replace "^Microsoft ", ""); arch = $arch; virt = $virt}
$cores = [Environment]::ProcessorCount
$memTotal = [int64]$os.TotalVisibleMemorySize * 1024
$cpuCounter = New-Object System.Diagnostics.PerformanceCounter("Processor", "% Processor Time", "_Total")
$memCounter = New-Object System.Diagnostics.PerformanceCounter("Memory", "Available Bytes")
[void]$cpuCounter.NextValue()

$info = @{}  # pid -> @{parent; cmd} (asked of CIM once)
function Learn($ids) {
    if (-not $ids.Count) { return }
    # A few: asked by id; many (the first sample): the whole list at once.
    $q = if ($ids.Count -le 16) { @{Filter = (($ids | ForEach-Object { "ProcessId=$_" }) -join " OR ")} } else { @{} }
    foreach ($p in (Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, CommandLine @q)) {
        $info[[int]$p.ProcessId] = @{parent = [int]$p.ParentProcessId; cmd = [string]$p.CommandLine}
    }
    foreach ($i in $ids) { if (-not $info.ContainsKey($i)) { $info[$i] = @{parent = 0; cmd = ""} } }
}
function NetBytes {
    $rx = 0; $tx = 0
    foreach ($n in [System.Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces()) {
        if ($n.NetworkInterfaceType -eq "Loopback" -or $n.OperationalStatus -ne "Up") { continue }
        $s = $n.GetIPStatistics(); $rx += $s.BytesReceived; $tx += $s.BytesSent
    }
    return @($rx, $tx)
}
$me = Get-Process -Id $PID
$prevCpu = @{}; $prevNet = NetBytes; $prevT = [DateTime]::UtcNow; $prevSelf = $me.TotalProcessorTime.TotalSeconds
$end = [DateTime]::UtcNow.AddSeconds($LIFETIME)
$next = [DateTime]::UtcNow.AddSeconds($PERIOD)
while ($true) {
    $left = ($next - [DateTime]::UtcNow).TotalMilliseconds
    if ($left -gt 0) { Start-Sleep -Milliseconds ([int]$left) }
    if ([DateTime]::UtcNow -ge $end) { Emit @{bye = $true}; exit 0 }
    $next = $next.AddSeconds($PERIOD)
    $now = [DateTime]::UtcNow
    $dt = [Math]::Max(($now - $prevT).TotalSeconds, 0.001)
    $busy = [Math]::Min($cpuCounter.NextValue() / 100, 1)
    $procs = Get-Process
    $cpu = @{}; $rss = @{}; $name = @{}
    foreach ($p in $procs) { $cpu[$p.Id] = $p.TotalProcessorTime.TotalSeconds; $rss[$p.Id] = $p.WorkingSet64; $name[$p.Id] = $p.ProcessName }
    Learn @($procs | Where-Object { -not $info.ContainsKey($_.Id) } | ForEach-Object { $_.Id })
    foreach ($k in @($info.Keys)) { if (-not $cpu.ContainsKey($k)) { $info.Remove($k) } }
    $children = @{}
    foreach ($k in $cpu.Keys) { $pp = $info[$k].parent; if (-not $children.ContainsKey($pp)) { $children[$pp] = New-Object System.Collections.ArrayList }; [void]$children[$pp].Add($k) }
    # The farm's: each Windows job (a cmd running a batch file under the farm's directory: farm-agent's
    # windows_job) with its tree, and WSL's VM (the farm's Linux on this machine).
    $farmPids = New-Object System.Collections.Generic.HashSet[int]
    $jobs = @(); $compiles = 0
    foreach ($k in $cpu.Keys) {
        $c = $info[$k].cmd
        $isJob = $name[$k] -eq "cmd" -and $c -match '\\rf\\' -and $c -match '\.(bat|cmd)'
        if ($name[$k] -like "vmmem*") { [void]$farmPids.Add($k) }
        if (-not $isJob -or $farmPids.Contains($k)) { continue }
        $tree = New-Object System.Collections.Generic.HashSet[int]; $stack = New-Object System.Collections.Stack; $stack.Push($k)
        while ($stack.Count) { $q = $stack.Pop(); if ($tree.Add($q)) { if ($children.ContainsKey($q)) { foreach ($ch in $children[$q]) { $stack.Push($ch) } } } }
        $jc = 0; $jr = 0
        foreach ($q in $tree) {
            [void]$farmPids.Add($q)
            $jc += [Math]::Max($cpu[$q] - ($(if ($prevCpu.ContainsKey($q)) { $prevCpu[$q] } else { 0 })), 0); $jr += $rss[$q]
            if ($name[$q] -eq "rustc") { $compiles++ }
        }
        $jobs += @{kind = "windows"; cmd = ($c -replace '^.*\\', '').Trim('"', ' '); checkout = ""; cpu = [Math]::Round($jc / $dt, 2); rss = $jr; pid = $k}
    }
    $farm = 0; $farmRss = 0
    foreach ($q in $farmPids) { $farm += [Math]::Max($cpu[$q] - ($(if ($prevCpu.ContainsKey($q)) { $prevCpu[$q] } else { 0 })), 0); $farmRss += $rss[$q] }
    $farm = [Math]::Min($farm / $dt / $cores, $busy)
    $me.Refresh(); $selfCpu = $me.TotalProcessorTime.TotalSeconds
    $probe = [Math]::Min(($selfCpu - $prevSelf) / $dt / $cores, [Math]::Max($busy - $farm, 0))
    $net = NetBytes
    $c = New-Object System.IO.DriveInfo("C")
    $avail = [int64]$memCounter.NextValue()
    Emit @{t = ([DateTimeOffset]$now).ToUnixTimeMilliseconds() / 1000.0; os = "windows"; cores = $cores; model = $cpuName; system = $system
        cpu = @{busy = [Math]::Round($busy, 4); farm = [Math]::Round($farm, 4); fleet = 0; probe = [Math]::Round($probe, 4); other = [Math]::Round([Math]::Max($busy - $farm - $probe, 0), 4)}
        load = $null; mem = @{total = $memTotal; available = $avail; used = $memTotal - $avail; farm_rss = $farmRss; installed = $installed}
        gpu = $null; disk = @{total = $c.TotalSize; free = $c.AvailableFreeSpace}
        net = @{rx = ($net[0] - $prevNet[0]) / $dt; tx = ($net[1] - $prevNet[1]) / $dt}
        jobs = $jobs; compiles = $compiles; fleet_jobs = 0; idle_s = $null
        probe_cpu = [Math]::Round(($selfCpu - $prevSelf) / $dt, 4)}
    $prevCpu = $cpu; $prevNet = $net; $prevT = $now; $prevSelf = $selfCpu
}
