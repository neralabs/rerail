-- A follow-up kept for later, commented out: it drops nothing
-- ALTER TABLE "User" DROP COLUMN "email";
/* DROP TABLE "Session"; */

-- DropTable: the name list ends where a comment starts
DROP TABLE "Gone"
-- "User" stays
;

-- DropTable: a view of the same name replaces ModelStats below, and Prisma reads the view
DROP TABLE "ModelStats";

-- AlterTable: "prompt" lives on only as an output name in raw SQL (`meta->>'prompt' prompt`)
ALTER TABLE "User" DROP COLUMN "prompt";

-- AlterTable
ALTER TABLE "posts" DROP COLUMN "title"; -- caught

-- CreateView: the model ModelStats now reads this view
CREATE OR REPLACE VIEW "ModelStats" AS SELECT "id", 1 AS "count" FROM "User";
