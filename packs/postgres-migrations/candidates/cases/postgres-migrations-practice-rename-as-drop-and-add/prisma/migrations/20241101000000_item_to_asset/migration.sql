-- DropForeignKey
ALTER TABLE "Note" DROP CONSTRAINT "Note_itemId_fkey";

-- AlterTable
ALTER TABLE "Item" RENAME TO "Asset";

-- AlterTable
ALTER TABLE "Note" DROP COLUMN "itemId", -- caught
ADD COLUMN     "assetId" TEXT NOT NULL;

-- AddForeignKey
ALTER TABLE "Note" ADD CONSTRAINT "Note_assetId_fkey" FOREIGN KEY ("assetId") REFERENCES "Asset"("id") ON DELETE CASCADE ON UPDATE CASCADE;
