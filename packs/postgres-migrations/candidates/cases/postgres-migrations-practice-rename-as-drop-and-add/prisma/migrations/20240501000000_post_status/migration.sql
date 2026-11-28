-- DropForeignKey
ALTER TABLE "Post" DROP CONSTRAINT "Post_ownerId_fkey";

-- AlterTable
ALTER TABLE "Post" DROP COLUMN "state"; -- caught

-- AlterTable
ALTER TABLE "Post" DROP COLUMN "ownerId", -- caught
ADD COLUMN     "authorId" TEXT;

-- AlterTable
ALTER TABLE "Post" ADD COLUMN     "status" "PostState" NOT NULL DEFAULT 'DRAFT';
