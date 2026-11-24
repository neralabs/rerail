import { prisma } from "./db";

// A data access function named after the app's own "session" model: no auth check.
export async function createWorkoutSession(userId: string) {
  return prisma.workoutSession.create({ data: { userId } });
}
