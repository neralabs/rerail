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
