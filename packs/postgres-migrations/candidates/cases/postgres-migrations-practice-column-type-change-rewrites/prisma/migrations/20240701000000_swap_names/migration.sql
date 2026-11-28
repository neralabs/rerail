-- AlterTable: "name" is now the former "title" column, whose history the rule does not follow
ALTER TABLE "Event" RENAME COLUMN "name" TO "label";
ALTER TABLE "Event" RENAME COLUMN "title" TO "name";
ALTER TABLE "Event" ALTER COLUMN "name" SET DATA TYPE VARCHAR(20);
