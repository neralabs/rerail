-- Two TEXT columns dropped and one added: "ip" is not a name like "zip" (too short to count), so no pair is unique
ALTER TABLE "Address" DROP COLUMN "ip",
DROP COLUMN "note",
ADD COLUMN "zip" TEXT;

-- AlterTable: neither name is an id (a name ending in Identifier is not one), so this is a rename
ALTER TABLE "Deployment" DROP COLUMN "externalIdentifier", -- caught
ADD COLUMN "externalRef" TEXT;
