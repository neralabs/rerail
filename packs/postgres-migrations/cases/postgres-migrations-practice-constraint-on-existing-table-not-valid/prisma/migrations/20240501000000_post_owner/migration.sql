/*
  Warnings:

  - Added the required column `ownerId` to the `Post` table without a default value. This is not possible if the table is not empty.

*/
-- AlterTable
ALTER TABLE "Post" ADD COLUMN "ownerId" TEXT NOT NULL;

-- AddForeignKey
ALTER TABLE "Post" ADD CONSTRAINT "Post_ownerId_fkey" FOREIGN KEY ("ownerId") REFERENCES "User"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AlterTable
ALTER TABLE "Comment" ADD COLUMN "editedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP, -- caught
ADD CONSTRAINT "Comment_editedAt_check" CHECK ("editedAt" > '2000-01-01');
