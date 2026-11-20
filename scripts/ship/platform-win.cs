// Windows's pinning, caps and measurement for the shippability bar (scripts/ship/platform.ts): one command's whole
// process tree in a job object, which holds it to the given CPUs (job affinity), caps its committed memory, and ends it
// at the time cap; the job's accounting gives CPU time, the root process its peak working set, and the per-CPU busy
// time over the run shows whether the pinned CPUs' idle siblings stayed idle.
//
//   shipbar-launch run <result.json> <cpus a,b> <idle siblings a,b> <memory cap bytes> <time cap s> -- <command> [args...]
//   shipbar-launch topology        each physical core's logical CPUs, as JSON: [[0,1],[2,3],...]
//
// The process starts suspended and joins the job before its first instruction, so every thread and child it makes is
// pinned and capped. It writes to the launcher's own stdout and stderr. C# 5, for the .NET Framework's csc, which every
// Windows has; platform.ts compiles it on first use. Writes to <result.json>:
//   { exit, killedBy: "time" | "memory" | null, wallMs, userMs, sysMs, peakWorkingSetKB, peakCommitKB, pinnedBusy, siblingBusy }

using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

static class ShipbarLaunch
{
    [StructLayout(LayoutKind.Sequential)]
    struct BasicLimit
    {
        public long PerProcessUserTimeLimit, PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass, SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct IoCounters { public ulong ReadOps, WriteOps, OtherOps, ReadBytes, WriteBytes, OtherBytes; }

    [StructLayout(LayoutKind.Sequential)]
    struct ExtendedLimit
    {
        public BasicLimit Basic;
        public IoCounters Io;
        public UIntPtr ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct BasicAccounting
    {
        public long TotalUserTime, TotalKernelTime, ThisPeriodTotalUserTime, ThisPeriodTotalKernelTime;
        public uint TotalPageFaultCount, TotalProcesses, ActiveProcesses, TotalTerminatedProcesses;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct CompletionPort { public IntPtr CompletionKey, Port; }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo
    {
        public int cb;
        public string lpReserved, lpDesktop, lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2;
        public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInformation { public IntPtr hProcess, hThread; public int dwProcessId, dwThreadId; }

    [StructLayout(LayoutKind.Sequential)]
    struct MemoryCounters
    {
        public uint cb, PageFaultCount;
        public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage, QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct ProcessorPerformance { public long IdleTime, KernelTime, UserTime, DpcTime, InterruptTime; public uint InterruptCount; }

    [StructLayout(LayoutKind.Sequential)]
    struct LogicalProcessor { public UIntPtr ProcessorMask; public int Relationship; public ulong Reserved0, Reserved1; }

    const uint LimitAffinity = 0x10, LimitJobMemory = 0x200, LimitDieOnUnhandledException = 0x400, LimitKillOnJobClose = 0x2000;
    const int ExtendedLimitInformation = 9, BasicAccountingInformation = 1, AssociateCompletionPortInformation = 7;
    const uint MessageJobMemoryLimit = 10;
    const uint CreateSuspended = 0x4, CreateUnicodeEnvironment = 0x400;
    const uint WaitTimeout = 0x102, Infinite = 0xFFFFFFFF;

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref ExtendedLimit info, int length);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref CompletionPort info, int length);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool QueryInformationJobObject(IntPtr job, int infoClass, out ExtendedLimit info, int length, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool QueryInformationJobObject(IntPtr job, int infoClass, out BasicAccounting info, int length, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool TerminateJobObject(IntPtr job, uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr CreateIoCompletionPort(IntPtr file, IntPtr existing, IntPtr key, uint threads);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool GetQueuedCompletionStatus(IntPtr port, out uint bytes, out IntPtr key, out IntPtr overlapped, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcess(string application, StringBuilder commandLine, IntPtr processAttributes, IntPtr threadAttributes, bool inheritHandles, uint flags, IntPtr environment, string directory, ref StartupInfo startup, out ProcessInformation info);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr GetStdHandle(int which);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetHandleInformation(IntPtr handle, uint mask, uint flags);
    [DllImport("kernel32.dll")] static extern uint SetErrorMode(uint mode);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool K32GetProcessMemoryInfo(IntPtr process, out MemoryCounters counters, uint size);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool GetLogicalProcessorInformation(IntPtr buffer, ref uint length);
    [DllImport("ntdll.dll")] static extern int NtQuerySystemInformation(int infoClass, IntPtr buffer, int length, out int returned);

    static int Main(string[] args)
    {
        try
        {
            if (args.Length == 1 && args[0] == "topology") { Console.Out.Write(Topology()); return 0; }
            if (args.Length >= 8 && args[0] == "run" && args[6] == "--") return Run(args);
            Console.Error.WriteLine("usage: shipbar-launch run <result.json> <cpus> <siblings> <memory cap bytes> <time cap s> -- <command> [args...] | topology");
            return 2;
        }
        catch (Exception error)
        {
            Console.Error.WriteLine("shipbar-launch: " + error.Message);
            return 127;
        }
    }

    static Exception Failed(string what) { return new Win32Exception(Marshal.GetLastWin32Error(), what); }

    static int[] Cpus(string list)
    {
        var cpus = new List<int>();
        foreach (var part in list.Split(new[] { ',' }, StringSplitOptions.RemoveEmptyEntries)) cpus.Add(int.Parse(part, CultureInfo.InvariantCulture));
        return cpus.ToArray();
    }

    /// One argument as CommandLineToArgvW reads it back: quoted when it holds a space, tab or quote, or is empty.
    static string Quote(string arg)
    {
        if (arg.Length > 0 && arg.IndexOfAny(new[] { ' ', '\t', '"' }) < 0) return arg;
        var quoted = new StringBuilder("\"");
        int backslashes = 0;
        foreach (char c in arg)
        {
            if (c == '\\') { backslashes++; continue; }
            quoted.Append('\\', c == '"' ? backslashes * 2 + 1 : backslashes);
            backslashes = 0;
            quoted.Append(c);
        }
        quoted.Append('\\', backslashes * 2);
        return quoted.Append('"').ToString();
    }

    /// Per CPU, busy and total time (100 ns) since boot; kernel time includes idle time.
    static ProcessorPerformance[] Processors()
    {
        int count = Environment.ProcessorCount, size = Marshal.SizeOf(typeof(ProcessorPerformance));
        IntPtr buffer = Marshal.AllocHGlobal(size * count);
        try
        {
            int returned;
            if (NtQuerySystemInformation(8, buffer, size * count, out returned) != 0) return new ProcessorPerformance[0];
            var all = new ProcessorPerformance[returned / size];
            for (int i = 0; i < all.Length; i++) all[i] = (ProcessorPerformance)Marshal.PtrToStructure(new IntPtr(buffer.ToInt64() + i * size), typeof(ProcessorPerformance));
            return all;
        }
        finally { Marshal.FreeHGlobal(buffer); }
    }

    static double Busiest(ProcessorPerformance[] before, ProcessorPerformance[] after, int[] cpus)
    {
        double most = 0;
        foreach (int cpu in cpus)
        {
            if (cpu >= before.Length || cpu >= after.Length) continue;
            long total = (after[cpu].KernelTime + after[cpu].UserTime) - (before[cpu].KernelTime + before[cpu].UserTime);
            long idle = after[cpu].IdleTime - before[cpu].IdleTime;
            if (total > 0) most = Math.Max(most, (double)(total - idle) / total);
        }
        return Math.Round(most, 3);
    }

    static string Topology()
    {
        uint length = 0;
        GetLogicalProcessorInformation(IntPtr.Zero, ref length);
        IntPtr buffer = Marshal.AllocHGlobal((int)length);
        try
        {
            if (!GetLogicalProcessorInformation(buffer, ref length)) throw Failed("GetLogicalProcessorInformation");
            int size = Marshal.SizeOf(typeof(LogicalProcessor));
            var cores = new List<string>();
            for (int offset = 0; offset + size <= length; offset += size)
            {
                var entry = (LogicalProcessor)Marshal.PtrToStructure(new IntPtr(buffer.ToInt64() + offset), typeof(LogicalProcessor));
                if (entry.Relationship != 0) continue;
                var threads = new List<string>();
                ulong mask = entry.ProcessorMask.ToUInt64();
                for (int cpu = 0; cpu < 64; cpu++) if ((mask & (1UL << cpu)) != 0) threads.Add(cpu.ToString(CultureInfo.InvariantCulture));
                cores.Add("[" + string.Join(",", threads) + "]");
            }
            return "[" + string.Join(",", cores) + "]";
        }
        finally { Marshal.FreeHGlobal(buffer); }
    }

    static IntPtr Inheritable(int which)
    {
        IntPtr handle = GetStdHandle(which);
        if (handle != IntPtr.Zero && handle != new IntPtr(-1)) SetHandleInformation(handle, 1, 1);
        return handle;
    }

    static int Run(string[] args)
    {
        string result = args[1];
        int[] cpus = Cpus(args[2]), siblings = Cpus(args[3]);
        ulong memoryCap = ulong.Parse(args[4], CultureInfo.InvariantCulture);
        uint timeCapMs = checked(uint.Parse(args[5], CultureInfo.InvariantCulture) * 1000);
        var commandLine = new StringBuilder();
        for (int i = 7; i < args.Length; i++) commandLine.Append(i == 7 ? "" : " ").Append(Quote(args[i]));

        // No error dialog for a crash: a crashed run exits with its code, as on Linux.
        SetErrorMode(0x0001 | 0x0002 | 0x8000);
        IntPtr job = CreateJobObject(IntPtr.Zero, null);
        if (job == IntPtr.Zero) throw Failed("CreateJobObject");
        ulong affinity = 0;
        foreach (int cpu in cpus) affinity |= 1UL << cpu;
        var limit = new ExtendedLimit();
        limit.Basic.LimitFlags = LimitAffinity | LimitJobMemory | LimitDieOnUnhandledException | LimitKillOnJobClose;
        limit.Basic.Affinity = new UIntPtr(affinity);
        limit.JobMemoryLimit = new UIntPtr(memoryCap);
        if (!SetInformationJobObject(job, ExtendedLimitInformation, ref limit, Marshal.SizeOf(typeof(ExtendedLimit)))) throw Failed("SetInformationJobObject (limits)");
        // The job reports reaching its memory limit on this port.
        var port = new CompletionPort { CompletionKey = job, Port = CreateIoCompletionPort(new IntPtr(-1), IntPtr.Zero, IntPtr.Zero, 1) };
        if (port.Port == IntPtr.Zero || !SetInformationJobObject(job, AssociateCompletionPortInformation, ref port, Marshal.SizeOf(typeof(CompletionPort)))) throw Failed("SetInformationJobObject (completion port)");

        var startup = new StartupInfo();
        startup.cb = Marshal.SizeOf(typeof(StartupInfo));
        startup.dwFlags = 0x100; // STARTF_USESTDHANDLES
        startup.hStdInput = Inheritable(-10);
        startup.hStdOutput = Inheritable(-11);
        startup.hStdError = Inheritable(-12);

        ProcessorPerformance[] before = Processors();
        var clock = Stopwatch.StartNew();
        ProcessInformation process;
        if (!CreateProcess(null, commandLine, IntPtr.Zero, IntPtr.Zero, true, CreateSuspended | CreateUnicodeEnvironment, IntPtr.Zero, null, ref startup, out process)) throw Failed("CreateProcess " + commandLine);
        if (!AssignProcessToJobObject(job, process.hProcess)) { TerminateJobObject(job, 1); throw Failed("AssignProcessToJobObject"); }
        ResumeThread(process.hThread);
        string killedBy = null;
        if (WaitForSingleObject(process.hProcess, timeCapMs) == WaitTimeout)
        {
            TerminateJobObject(job, 137);
            WaitForSingleObject(process.hProcess, Infinite);
            killedBy = "time";
        }
        clock.Stop();
        ProcessorPerformance[] after = Processors();
        uint exit;
        GetExitCodeProcess(process.hProcess, out exit);

        BasicAccounting accounting;
        QueryInformationJobObject(job, BasicAccountingInformation, out accounting, Marshal.SizeOf(typeof(BasicAccounting)), IntPtr.Zero);
        ExtendedLimit used;
        QueryInformationJobObject(job, ExtendedLimitInformation, out used, Marshal.SizeOf(typeof(ExtendedLimit)), IntPtr.Zero);
        MemoryCounters memory;
        ulong peakWorkingSet = K32GetProcessMemoryInfo(process.hProcess, out memory, (uint)Marshal.SizeOf(typeof(MemoryCounters))) ? memory.PeakWorkingSetSize.ToUInt64() : 0;
        uint message;
        IntPtr key, overlapped;
        while (killedBy == null && GetQueuedCompletionStatus(port.Port, out message, out key, out overlapped, 0))
        {
            if (message == MessageJobMemoryLimit && exit != 0) killedBy = "memory";
        }
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
        CloseHandle(job);

        var json = string.Format(
            CultureInfo.InvariantCulture,
            "{{\"exit\":{0},\"killedBy\":{1},\"wallMs\":{2},\"userMs\":{3},\"sysMs\":{4},\"peakWorkingSetKB\":{5},\"peakCommitKB\":{6},\"pinnedBusy\":{7},\"siblingBusy\":{8}}}",
            (int)exit,
            killedBy == null ? "null" : "\"" + killedBy + "\"",
            clock.ElapsedMilliseconds,
            accounting.TotalUserTime / 10000,
            accounting.TotalKernelTime / 10000,
            peakWorkingSet / 1024,
            used.PeakProcessMemoryUsed.ToUInt64() / 1024,
            Busiest(before, after, cpus).ToString("0.###", CultureInfo.InvariantCulture),
            Busiest(before, after, siblings).ToString("0.###", CultureInfo.InvariantCulture));
        File.WriteAllText(result, json);
        return 0;
    }
}
