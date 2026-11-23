import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { cmdletsWithoutModule, POWERSHELL_ALIASES } from "./powershell-commands.js";

describe("the cmdlets a script names without their module", () => {
  it("reads a verb of two capitalised words, as ConvertTo-Json's", () => {
    expect(cmdletsWithoutModule("[Console]::Out.Write((@{ found = $true } | ConvertTo-Json -Compress))")).toEqual(["ConvertTo-Json"]);
  });

  it("reads ConvertFrom-Json, and not the name with its module before it", () => {
    expect(cmdletsWithoutModule([
      "$request = [Console]::In.ReadToEnd() | ConvertFrom-Json",
      "$answer = $text | Microsoft.PowerShell.Utility\\ConvertFrom-Json",
    ].join("\n"))).toEqual(["ConvertFrom-Json"]);
  });

  // PowerShell finds a command without regard to case, so `out-null` makes it read the modules as `Out-Null` does.
  it("reads a name in any case, and not one in any case with its module before it", () => {
    expect(cmdletsWithoutModule("$null = (get-process -Id 1) | out-null; GET-ACL -LiteralPath $p; microsoft.powershell.utility\\convertto-json"))
      .toEqual(["get-process", "out-null", "GET-ACL"]);
  });

  it.each(POWERSHELL_ALIASES)("reads the alias %s after a pipe", alias => {
    expect(cmdletsWithoutModule(`$found = @($items | ${alias} { $_ })`)).toEqual([alias]);
  });

  // Get-Alias lists a module's own aliases (gin, for Get-ComputerInfo) only once the module is loaded.
  it.runIf(process.platform === "win32")("holds every alias Windows PowerShell defines for a command of its modules", () => {
    const script = [
      "Import-Module Microsoft.PowerShell.Management, Microsoft.PowerShell.Utility, Microsoft.PowerShell.Security",
      "$commands = (Get-Command -ListImported | Where-Object { $_.ModuleName }).Name",
      "(Get-Alias | Where-Object { $commands -contains $_.Definition }).Name",
    ].join("; ");
    const powershell = join(process.env.SystemRoot ?? String.raw`C:\Windows`, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
    const defined = execFileSync(powershell, ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script], {
      encoding: "utf8", timeout: 30_000,
    }).split(/\r?\n/u).filter(name => name !== "");
    expect(defined.sort()).toEqual([...POWERSHELL_ALIASES].sort());
    // Windows PowerShell can take seconds to start on a cold machine.
  }, 60_000);

  // PowerShell ends a command's name at a brace or a parenthesis as at a space: `$items | %{ $_ }` runs ForEach-Object.
  it("reads an alias written right before a brace or a parenthesis", () => {
    expect(cmdletsWithoutModule("$found = $items | %{ $_ } | ?{ $_.Ok } | where{ $_ }\necho('done')"))
      .toEqual(["%", "?", "where", "echo"]);
  });

  it("reads an alias after return, throw, exit, in, and the dot that runs a command in the caller's scope", () => {
    expect(cmdletsWithoutModule("foreach ($p in gps) { . ls }\nif ($p) { return select }\nthrow sort\nexit measure"))
      .toEqual(["gps", "ls", "select", "sort", "measure"]);
  });

  it("reads no alias in a comment, and reads one after a comment that holds an apostrophe", () => {
    expect(cmdletsWithoutModule([
      "# Don't sort here",
      "$first = $items | select -First 1 # it's the first",
      "<#",
      "gps -Id 1",
      "#>",
      "$last = 'x' | sort",
    ].join("\n"))).toEqual(["select", "sort"]);
  });

  // An emoji is two UTF-16 units, and each becomes a space, so an alias after it reads at its own place.
  it("reads an alias at its place after a string that holds an emoji", () => {
    expect(cmdletsWithoutModule("$mark = '\u{1F600}'; $items | sort | Select-Object -First 1")).toEqual(["sort", "Select-Object"]);
  });

  it("reads an alias in any case at a statement's start, after a semicolon, an equals sign or a parenthesis, in script order", () => {
    expect(cmdletsWithoutModule("GPS -Id 1\n  sleep 1; $list = ls\n$n = (Measure).Count | Select-Object -First 1"))
      .toEqual(["GPS", "sleep", "ls", "Measure", "Select-Object"]);
  });

  it("reads no alias in a string, a here-string, a parameter or a .NET call, and not the foreach keyword", () => {
    expect(cmdletsWithoutModule([
      "$ErrorActionPreference = \"Stop\"",
      "Microsoft.PowerShell.Utility\\Add-Type -TypeDefinition @\"",
      "public class Store { public string Value { get; set; } }",
      "\"@",
      "[Console]::Out.Write('select sort')",
      "foreach ($item in $items) { [Console]::Out.Write($item.Type) }",
      "switch ($request.operation) { \"read\" { $value = [Store]::Read($request.target) } default { throw \"no type\" } }",
    ].join("\n"))).toEqual([]);
  });
});
