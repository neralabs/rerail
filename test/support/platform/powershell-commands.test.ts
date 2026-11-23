import { describe, expect, it } from "vitest";
import { cmdletsWithoutModule } from "./powershell-commands.js";

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
});
