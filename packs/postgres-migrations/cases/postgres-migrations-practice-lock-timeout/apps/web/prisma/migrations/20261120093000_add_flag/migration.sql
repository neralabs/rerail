-- CreateTable
CREATE TABLE "Tag" (
    "id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    CONSTRAINT "Tag_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE UNIQUE INDEX "Tag_name_key" ON "Tag"("name");

-- AlterTable
ALTER TABLE "User" ADD COLUMN "flag" BOOLEAN; -- caught

-- AlterTable
ALTER TABLE "Post" ADD COLUMN "tagId" TEXT;
