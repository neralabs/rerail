"use server";

import { prisma } from "@/lib/db";
import { auth } from "@/lib/auth";
import { fetchQuestion } from "@/lib/questions";

// uid is Roadmap's @id in the schema: the unique lookup rejects an object roadmapUid before the filter, with an AND
// beside it. userUid was only inside the AND, which takes filters: it is still open.
export async function roadmapAnswers(roadmapUid: string, userUid: string) {
  const roadmap = await prisma.roadmap.findUnique({ where: { uid: roadmapUid, AND: { userUid } } });
  if (!roadmap) return null;
  return prisma.answer.findMany({
    where: {
      roadmapUid,
      userUid, // caught
    },
  });
}

// The helper's unique lookup by the question's @id, beside a relation filter, ran first.
export async function answerQuestion(questionUid: string) {
  const question = await fetchQuestion(questionUid);
  if (!question) throw new Error("Question not found");
  return prisma.userAnswer.findFirst({ where: { questionUid } });
}

// The lookup reads the row; the unique update by the same @id after it (beside userId) rejects an object before
// anything is written or returned.
export async function editSolution(id: number, title: string) {
  const session = await auth();
  const solution = await prisma.solution.findFirstOrThrow({ where: { id } });
  if (solution.userId !== session.user.id) throw new Error("Not allowed");
  await prisma.solution.update({ where: { id, userId: session.user.id }, data: { title } });
}

// A bulk delete before the unique one: it has removed rows by the time the object reaches the delete.
export async function removeSolution(id: number) {
  await prisma.comment.deleteMany({ where: { solutionId: id } }); // caught
  await prisma.solution.delete({ where: { id } });
}

// The read returns from its own branch, where the unique update after it never runs.
export async function solutionOrRename(id: number, title?: string) {
  if (!title) {
    return prisma.solution.findFirst({ where: { id } }); // caught
  }
  await prisma.solution.update({ where: { id }, data: { title } });
}

// In an interactive transaction, the unique update after the bulk delete rolls it back when it throws.
export async function regenerate(questionUid: string) {
  await prisma.$transaction(async (tx) => {
    await tx.answerOption.deleteMany({ where: { questionUid } });
    await tx.question.update({ where: { uid: questionUid }, data: { regenerated: true } });
  });
}

// A batch transaction fails whole when one of its queries does: the unique update by the same id rejects an object.
export async function unbanUser(userId: string) {
  return prisma.$transaction([
    prisma.solution.updateMany({ where: { userId }, data: { status: "ACTIVE" } }),
    prisma.user.update({ where: { id: userId }, data: { status: "ACTIVE" } }),
  ]);
}

// Two bulk writes batched: neither rejects an object.
export async function hideUser(userId: string) {
  return prisma.$transaction([
    prisma.solution.updateMany({ where: { userId }, data: { visible: false } }), // caught
    prisma.comment.updateMany({ where: { userId }, data: { visible: false } }), // caught
  ]);
}

// Strictly equal to the session's user id, or it returns: an object never gets past the guard. challengeId is not
// compared: it is still open.
export async function toggleBookmark(challengeId: number, userId: string) {
  const session = await auth();
  if (!session?.user?.id || userId !== session.user.id) return "unauthorized";
  return prisma.bookmark.findFirst({
    where: {
      challengeId, // caught
      userId,
    },
  });
}

// The comparison only logs: the object goes on to the filter.
export async function bookmarksFor(userId: string) {
  const session = await auth();
  if (userId !== session.user.id) console.warn("bookmarks of another user");
  return prisma.bookmark.findMany({ where: { userId } }); // caught
}
