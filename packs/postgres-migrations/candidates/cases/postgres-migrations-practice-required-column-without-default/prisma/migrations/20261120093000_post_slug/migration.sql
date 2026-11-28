-- AlterTable
ALTER TABLE "Post" ADD COLUMN     "slug" TEXT NOT NULL, -- caught
ADD COLUMN     "updatedAt" TIMESTAMP(3) NOT NULL; -- caught

-- AlterTable
ALTER TABLE post ADD COLUMN code TEXT NOT NULL; -- caught
