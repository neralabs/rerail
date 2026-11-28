/*
  Warnings:

  - Made the column `phone` on table `User` required. This step will fail if there are existing NULL values in that column.
  - Made the column `authorId` on table `Post` required. This step will fail if there are existing NULL values in that column.
  - Made the column `slug` on table `Post` required. This step will fail if there are existing NULL values in that column.

*/
-- AlterTable
ALTER TABLE "User" ALTER COLUMN "phone" SET NOT NULL; -- caught

-- AlterTable
ALTER TABLE "Post" ALTER COLUMN "authorId" SET NOT NULL, -- caught
ALTER COLUMN "slug" SET NOT NULL;

-- Step 3: the validated CHECK proves no NULL exists, so Postgres skips the scan.
ALTER TABLE "Account" ALTER COLUMN "provider" SET NOT NULL;

-- The CHECK on this column was never validated.
ALTER TABLE "Session" ALTER COLUMN "userAgent" SET NOT NULL; -- caught

-- Making a column nullable does not scan.
ALTER TABLE "User" ALTER COLUMN "name" DROP NOT NULL;
