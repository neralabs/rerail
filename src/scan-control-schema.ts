import { z } from "zod/v4";

export const policySettingsSchema = z.strictObject({
  scope: z.enum(["repository", "touched-files", "ai-output"]),
  mode: z.enum(["log", "resteer"]),
});
export type PolicySettings = z.infer<typeof policySettingsSchema>;
export const policySettingsPatchSchema = policySettingsSchema.partial().strict();
export const repositoryDiagnosticSchema = z.strictObject({
  sessionId: z.string(),
  runId: z.string().nullable(),
  status: z.enum(["running", "completed", "failed", "interrupted"]),
  startedAt: z.number(),
  finishedAt: z.number().nullable(),
  error: z.string().nullable(),
});
export type RepositoryDiagnostic = z.infer<typeof repositoryDiagnosticSchema>;
export const scanInputSchema = z.strictObject({
  scope: z.enum(["repository", "git-diff"]).optional(),
});
export type ScanInput = z.infer<typeof scanInputSchema>;
export const scanRequestSchema = z.strictObject({
  id: z.string(), sessionId: z.string(), workspaceRoot: z.string(),
  scope: z.enum(["repository", "touched-files", "ai-output"]), mode: z.enum(["log", "resteer"]),
  paths: z.array(z.string()), status: z.enum(["queued", "running", "completed", "failed", "interrupted"]),
  createdAt: z.number(), startedAt: z.number().nullable(), finishedAt: z.number().nullable(),
  runIds: z.array(z.string()), error: z.string().nullable(),
});
export type ScanRequest = z.infer<typeof scanRequestSchema>;
export const policyControlsSchema = z.strictObject({
  workspaceRoot: z.string(), settings: policySettingsSchema, settingsPath: z.string(),
  delivery: z.enum(["background", "next-event"]), active: z.boolean(),
  repositoryDiagnostic: repositoryDiagnosticSchema.nullable(),
});
export type PolicyControls = z.infer<typeof policyControlsSchema>;
