-- A follow-up kept for later, commented out: it drops nothing
-- ALTER TABLE "User" DROP COLUMN "email";
/* DROP TABLE "Session"; */

-- DropTable: the name list ends where a comment starts
DROP TABLE "Gone"
-- "User" stays
;

-- DropTable: ModelStats is a `/// @view` model now, a view that replaced this table
DROP TABLE "ModelStats";

-- AlterTable: "prompt" lives on only as an output name in raw SQL (`meta->>'prompt' prompt`)
ALTER TABLE "User" DROP COLUMN "prompt";

-- AlterTable
ALTER TABLE "posts" DROP COLUMN "title"; -- caught
