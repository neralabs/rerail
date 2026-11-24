/*
  Warnings:

  - You are about to drop the column `legacyRole` on the `User` table. All the data in the column will be lost.

*/
-- AlterTable
ALTER TABLE "User" DROP COLUMN "legacyRole"; -- caught

-- AlterTable
ALTER TABLE "posts" DROP COLUMN "author_name", -- caught
DROP COLUMN "oldFlag";

-- AlterTable
ALTER TABLE "User" DROP COLUMN "tier"; -- caught

-- AlterTable
ALTER TABLE "Account" DROP COLUMN "legacyRole", -- caught
DROP COLUMN "tier";

-- AlterTable
ALTER TABLE "User" DROP COLUMN "nickname";

-- AlterTable
ALTER TABLE "posts" ALTER COLUMN "title" DROP DEFAULT,
DROP CONSTRAINT "posts_title_key";

-- DropTable
DROP TABLE "Session"; -- caught

-- DropTable
DROP TABLE "AuditLog";

-- AlterTable (a type change Prisma cannot cast: the column is dropped and added again)
ALTER TABLE "Account" DROP COLUMN "provider",
ADD COLUMN     "provider" TEXT NOT NULL DEFAULT 'email';
