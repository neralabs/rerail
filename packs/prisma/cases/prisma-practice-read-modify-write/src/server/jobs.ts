import { db } from "@/lib/db";

// A worker claimed the job (lockId set by an earlier updateMany); only the claim holder writes it, and the write
// releases the claim.
export async function finishJob(jobId: string, lockId: string) {
  const job = await db.job.findUnique({ where: { id: jobId } });
  if (!job || job.lockId !== lockId) return;
  const attempt = job.attempts + 1;
  await db.job.update({
    where: { id: job.id },
    data: { attempts: attempt, lockId: null, lockedAt: null },
  });
}

// The claim holder is part of the filter.
export async function retryJob(jobId: string, lockId: string) {
  const job = await db.job.findUniqueOrThrow({ where: { id: jobId } });
  await db.job.update({ where: { id: job.id, lockId }, data: { attempts: job.attempts + 1 } });
}

// No claim, no guard: two retries at once count one attempt.
export async function bumpAttempts(jobId: string) {
  const job = await db.job.findUniqueOrThrow({ where: { id: jobId } });
  await db.job.update({ where: { id: job.id }, data: { attempts: job.attempts + 1 } }); // caught
}
