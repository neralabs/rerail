-- A real rename: the column keeps its data.
ALTER TABLE "Profile" RENAME COLUMN "website" TO "homepage";

-- Different types: a new column, not a rename.
ALTER TABLE "Profile" DROP COLUMN "legacyFlag",
ADD COLUMN     "status" TEXT NOT NULL DEFAULT 'active';

-- The same column re-created (a type change written by hand): not a rename.
ALTER TABLE "Profile" DROP COLUMN "price",
ADD COLUMN     "price" TEXT;

-- The data is copied before the drop.
ALTER TABLE "Profile" ADD COLUMN "displayName" TEXT;
UPDATE "Profile" SET "displayName" = "nickname";
ALTER TABLE "Profile" DROP COLUMN "nickname";

-- A column no earlier migration defines (baselined history): its type is unknown.
ALTER TABLE "Account" DROP COLUMN "provider_id",
ADD COLUMN     "providerAccountId" TEXT;

-- A drop alone.
ALTER TABLE "Post" DROP COLUMN "heading";
