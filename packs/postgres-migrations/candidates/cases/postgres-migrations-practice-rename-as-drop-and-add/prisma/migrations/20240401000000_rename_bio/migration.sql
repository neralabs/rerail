/*
  Warnings:

  - You are about to drop the column `biograpy` on the `Profile` table. All the data in the column will be lost.
  - Added the required column `biography` to the `Profile` table without a default value. This is not possible if the table is not empty.

*/
-- AlterTable
ALTER TABLE "Profile" DROP COLUMN "biograpy", -- caught
ADD COLUMN     "biography" TEXT NOT NULL;
