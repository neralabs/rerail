import { z } from "zod/v4";

export const auditSettingsSchema = z.strictObject({
  scope: z.enum(["repository", "touched-files", "ai-output"]),
  mode: z.enum(["log", "steer"]),
});
export type AuditSettings = z.infer<typeof auditSettingsSchema>;
export const auditSettingsPatchSchema = auditSettingsSchema.partial().strict();
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
