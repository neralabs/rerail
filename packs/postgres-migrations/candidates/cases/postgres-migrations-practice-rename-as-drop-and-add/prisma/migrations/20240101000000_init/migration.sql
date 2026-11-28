-- CreateEnum
CREATE TYPE "PostState" AS ENUM ('DRAFT', 'PUBLISHED');

-- CreateTable
CREATE TABLE "Profile" (
    "id" SERIAL NOT NULL,
    "biograpy" TEXT NOT NULL,
    "legacyFlag" BOOLEAN NOT NULL DEFAULT false,
    "price" TEXT,
    "nickname" TEXT,
    "website" TEXT,

    CONSTRAINT "Profile_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "Post" (
    "id" TEXT NOT NULL,
    "state" "PostState" NOT NULL DEFAULT 'DRAFT',
    "title" VARCHAR(200) NOT NULL,
    "heading" VARCHAR(200),

    CONSTRAINT "Post_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "Session" (
    "id" TEXT NOT NULL,
    "expires" TIMESTAMP(3) NOT NULL,

    CONSTRAINT "Session_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "UserProfile" (
    "userId" INTEGER NOT NULL,
    "profileBio" TEXT,
    "profileURL" TEXT,

    CONSTRAINT "UserProfile_pkey" PRIMARY KEY ("userId")
);

-- CreateTable
CREATE TABLE "User" ("id" TEXT NOT NULL, CONSTRAINT "User_pkey" PRIMARY KEY ("id"));
CREATE TABLE "Member" ("id" TEXT NOT NULL, CONSTRAINT "Member_pkey" PRIMARY KEY ("id"));
CREATE TABLE "Item" ("id" TEXT NOT NULL, CONSTRAINT "Item_pkey" PRIMARY KEY ("id"));
CREATE TABLE "Comment" ("id" TEXT NOT NULL, "userId" TEXT, CONSTRAINT "Comment_pkey" PRIMARY KEY ("id"));
CREATE TABLE "Note" ("id" TEXT NOT NULL, "itemId" TEXT NOT NULL, CONSTRAINT "Note_pkey" PRIMARY KEY ("id"));

-- AddForeignKey
ALTER TABLE "Comment" ADD CONSTRAINT "Comment_userId_fkey" FOREIGN KEY ("userId") REFERENCES "User"("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "Note" ADD CONSTRAINT "Note_itemId_fkey" FOREIGN KEY ("itemId") REFERENCES "Item"("id") ON DELETE CASCADE ON UPDATE CASCADE;
CREATE TABLE "Job" ("id" TEXT NOT NULL, "error" TEXT, "output" TEXT, "label" TEXT, CONSTRAINT "Job_pkey" PRIMARY KEY ("id"));
CREATE TABLE "Run" ("id" TEXT NOT NULL, "queueName" TEXT, CONSTRAINT "Run_pkey" PRIMARY KEY ("id"));
