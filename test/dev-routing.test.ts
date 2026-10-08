import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { resolveRerailWorkspaceRoute, rerailDevLauncher } from "../src/dev-routing.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function instance(root: string, port: number): Promise<void> {
  await mkdir(join(root, ".nera"), { recursive: true });
  await writeFile(join(root, ".nera", "instance.env"), `NERA_RERAIL_ENABLED="true"\nNERA_RERAIL_HOST_PORT="${port}"\nNERA_RERAIL_PUBLIC_URL="http://127.0.0.1:${port}"\n`);
}

describe("Rerail development bridge routing", () => {
  it("selects the nearest checkout instance from a nested host cwd", async () => {
    const parent = await mkdtemp(join(tmpdir(), "rerail-dev-parent-"));
    const checkout = join(parent, "checkout");
    temporaryDirectories.push(parent);
    await instance(parent, 4811);
    await instance(checkout, 4812);
    await mkdir(join(checkout, "nested", "work"), { recursive: true });

    const route = await resolveRerailWorkspaceRoute(join(checkout, "nested", "work"));
    expect(route.kind).toBe("workspace");
    if (route.kind !== "workspace") throw new Error("Expected workspace route");
    expect(route.instance.workspaceRoot).toBe(checkout);
    expect(route.instance.url.href).toBe("http://127.0.0.1:4812/");
  });

  it("rejects an instance that could send host events off-machine", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-dev-public-"));
    temporaryDirectories.push(root);
    await mkdir(join(root, ".nera"));
    await writeFile(join(root, ".nera", "instance.env"), "NERA_RERAIL_ENABLED=true\nNERA_RERAIL_PUBLIC_URL=http://example.test:4737\n");

    await expect(resolveRerailWorkspaceRoute(root)).rejects.toThrow("explicit loopback HTTP address");
  });

  it("accepts an explicit IPv6 loopback origin", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-dev-ipv6-"));
    temporaryDirectories.push(root);
    await mkdir(join(root, ".nera"));
    await writeFile(join(root, ".nera", "instance.env"), "NERA_RERAIL_ENABLED=true\nNERA_RERAIL_PUBLIC_URL=http://[::1]:4737\n");

    const route = await resolveRerailWorkspaceRoute(root);
    expect(route.kind).toBe("workspace");
    if (route.kind !== "workspace") throw new Error("Expected workspace route");
    expect(route.instance.url.href).toBe("http://[::1]:4737/");
  });

  it("does not treat an ordinary checkout as a development instance", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-dev-none-"));
    temporaryDirectories.push(root);

    await expect(resolveRerailWorkspaceRoute(root)).resolves.toEqual({ kind: "host" });
  });

  it("keeps pre-Rerail repository operations checkouts on the managed installation", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-dev-legacy-"));
    temporaryDirectories.push(root);
    await mkdir(join(root, ".nera"));
    await writeFile(join(root, ".nera", "instance.env"), 'INSTANCE="legacy"\nNERA_WEB_HOST_PORT="30001"\n');

    await expect(resolveRerailWorkspaceRoute(root)).resolves.toEqual({ kind: "host" });
  });

  it("fails closed when an explicitly provisioned Rerail instance is incomplete", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-dev-incomplete-"));
    temporaryDirectories.push(root);
    await mkdir(join(root, ".nera"));
    await writeFile(join(root, ".nera", "instance.env"), 'NERA_RERAIL_ENABLED="true"\nNERA_RERAIL_HOST_PORT="4737"\n');

    await expect(resolveRerailWorkspaceRoute(root)).rejects.toThrow("lacks NERA_RERAIL_PUBLIC_URL");
  });

  it("keeps nested temporary worktrees disabled when only the application stack was provisioned", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-dev-opt-in-"));
    const worktree = join(root, "tmp", "baseline-gate-audit");
    temporaryDirectories.push(root);
    await instance(root, 4811);
    await writeFile(join(root, ".nera", "instance.env"), 'NERA_RERAIL_ENABLED="false"\nNERA_RERAIL_PUBLIC_URL="http://127.0.0.1:4811"\n');
    await mkdir(worktree, { recursive: true });

    await expect(resolveRerailWorkspaceRoute(worktree)).resolves.toEqual({
      kind: "disabled",
      workspaceRoot: root,
      configurationFile: join(root, ".nera", "instance.env"),
    });
  });

  it("launches checkout commands through the checkout's own ops.sh under an absolute shell", () => {
    expect(rerailDevLauncher({ workspaceRoot: "/checkout" }, ["status"])).toEqual({
      command: "/bin/bash",
      args: ["/checkout/scripts/ops.sh", "rerail-command", "status"],
    });
  });
});
