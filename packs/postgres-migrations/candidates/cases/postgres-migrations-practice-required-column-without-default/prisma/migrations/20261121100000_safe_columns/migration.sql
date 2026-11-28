-- AlterTable: a constant default fills the existing rows
ALTER TABLE "Post" ADD COLUMN     "views" INTEGER NOT NULL DEFAULT 0,
ADD COLUMN     "summary" TEXT,
ADD COLUMN     "wordCount" INTEGER GENERATED ALWAYS AS (length("title")) STORED NOT NULL,
ADD COLUMN     "rowNo" BIGINT GENERATED ALWAYS AS IDENTITY NOT NULL,
ADD COLUMN     "seq" BIGSERIAL NOT NULL;

-- CreateTable: a table created in this migration is empty
CREATE TABLE "Tag" (
    "id" TEXT NOT NULL,
    CONSTRAINT "Tag_pkey" PRIMARY KEY ("id")
);
ALTER TABLE "Tag" ADD COLUMN "label" TEXT NOT NULL;

-- emptied first, then the column is safe
TRUNCATE TABLE "Draft";
ALTER TABLE "Draft" ADD COLUMN "owner" TEXT NOT NULL;
DELETE FROM "Draft";
ALTER TABLE "Draft" ADD COLUMN "reviewer" TEXT NOT NULL;

-- serial has an implicit default
ALTER TABLE "Post" ADD COLUMN "legacyNo" SERIAL NOT NULL;

-- a key column: index-on-existing-table-blocks-writes reports it, not this rule
ALTER TABLE "Post" ADD COLUMN "ref" TEXT PRIMARY KEY;
