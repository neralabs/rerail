import { prisma } from "./db";
import { auth } from "./auth";

// A unique lookup by the question's @id, beside a relation filter that scopes it to the caller's roadmaps.
export async function fetchQuestion(questionUid: string) {
  const session = await auth();
  return prisma.question.findUnique({ where: { uid: questionUid, roadmap: { userUid: session.user.id } } });
}
