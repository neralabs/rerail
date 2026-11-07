/**
 * What a seeded scenario is, shared by the seeding process (which imports the product's stores) and the Playwright
 * side (which only reads the manifest it prints). Nothing here imports `src/`.
 */

/** The fixed instant every scenario is seeded against; the page clock is frozen to it. Thu Oct 22 2026, 15:00 UTC. */
export const SEED_NOW = Date.UTC(2026, 9, 22, 15, 0, 0);
export const MINUTE = 60_000;
export const HOUR = 60 * MINUTE;
export const DAY = 24 * HOUR;

export const SCENARIO_NAMES = ["no-packs", "packs-no-session", "mature", "multi-repo", "not-set-up", "stopped"] as const;
export type ScenarioName = (typeof SCENARIO_NAMES)[number];

export function isScenarioName(value: string): value is ScenarioName {
  return (SCENARIO_NAMES as readonly string[]).includes(value);
}

/** One repository a scenario recorded, with the checkout the dashboard is scoped to. */
export interface ScenarioRepository {
  readonly repositoryKey: string;
  readonly displayName: string;
  /** Physical path, as Rerail recorded it (`?checkout=`). */
  readonly checkoutPath: string;
  readonly state: "started" | "stopped" | "not-set-up";
  /** Rule packs added to the repository (whole repository, folder ""). */
  readonly packIds: readonly string[];
  /** Rules created by you (library rules) added to the repository. */
  readonly libraryRuleIds: readonly string[];
  /** Every rule with at least one recorded violation, sorted. */
  readonly violatingRuleIds: readonly string[];
  /** Every session, by start, oldest first. */
  readonly sessionIds: readonly string[];
  /** Sessions with no end yet. */
  readonly openSessionIds: readonly string[];
}

export interface ScenarioManifest {
  readonly scenario: ScenarioName;
  readonly now: number;
  /** The scenario's folder; everything below it is throwaway. */
  readonly root: string;
  /** `RERAIL_HOME`: holds `rerail.sqlite`, the installed packs and `daemon.json`. */
  readonly rerailHome: string;
  /** `HOME` for the daemon, so no coding agent config of the machine is read. */
  readonly userHome: string;
  readonly databasePath: string;
  /** The repository the daemon is started in and the dashboard is scoped to. */
  readonly primary: ScenarioRepository;
  /** The primary repository's sessions by start, oldest first (`primary.sessionIds`); empty when it has none. */
  readonly sessionIds: readonly string[];
  readonly repositories: readonly ScenarioRepository[];
}
