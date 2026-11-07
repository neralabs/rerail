import { z } from "zod/v4";
import { STEER_STRATEGIES } from "../adapter-contract.js";

const auditSettingsShape = {
  scope: z.enum(["repository", "touched-files", "ai-output"]),
  mode: z.enum(["log", "steer"]),
  steerStrategy: z.enum(STEER_STRATEGIES),
};
/** A row saved before `steerStrategy` existed reads as `message`, the strategy Rerail always used. */
export const auditSettingsSchema = z.strictObject({
  ...auditSettingsShape,
  steerStrategy: auditSettingsShape.steerStrategy.default("message"),
});
export type AuditSettings = z.infer<typeof auditSettingsSchema>;
/** Automatic audits report what an edit generated (`ai-output`), steer, and send the steer as its own message. */
export const DEFAULT_AUDIT_SETTINGS: Readonly<AuditSettings> = { scope: "ai-output", mode: "steer", steerStrategy: "message" };
/** Only the keys a patch names change: no default fills a key it leaves out. */
export const auditSettingsPatchSchema = z.strictObject(auditSettingsShape).partial().strict();
export type AuditSettingsPatch = z.infer<typeof auditSettingsPatchSchema>;
export const repositoryAuditSchema = z.strictObject({
  sessionId: z.string(),
  auditId: z.string().nullable(),
  status: z.enum(["running", "completed", "failed", "interrupted"]),
  startedAt: z.number(),
  finishedAt: z.number().nullable(),
  error: z.string().nullable(),
});
export type RepositoryAudit = z.infer<typeof repositoryAuditSchema>;
export const auditRequestInputSchema = z.strictObject({
  scope: z.enum(["repository", "git-diff"]).optional(),
});
export type AuditRequestInput = z.infer<typeof auditRequestInputSchema>;
export const auditRequestSchema = z.strictObject({
  id: z.string(), sessionId: z.string(), workspaceRoot: z.string(),
  scope: z.enum(["repository", "touched-files", "ai-output"]), mode: z.enum(["log", "steer"]),
  paths: z.array(z.string()), status: z.enum(["queued", "running", "completed", "failed", "interrupted"]),
  createdAt: z.number(), startedAt: z.number().nullable(), finishedAt: z.number().nullable(),
  auditIds: z.array(z.string()), error: z.string().nullable(),
});
export type AuditRequest = z.infer<typeof auditRequestSchema>;
export const auditControlsSchema = z.strictObject({
  workspaceRoot: z.string(), settings: auditSettingsSchema, settingsPath: z.string(),
  delivery: z.enum(["background", "next-event"]), active: z.boolean(),
  repositoryAudit: repositoryAuditSchema.nullable(),
});
export type AuditControls = z.infer<typeof auditControlsSchema>;
