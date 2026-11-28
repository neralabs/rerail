-- AlterTable: a restructure, not a rename. Two TEXT columns go and one comes, and the label goes while two
-- unrelated TEXT columns come: no drop pairs with a single add.
ALTER TABLE "Job" DROP COLUMN "error",
DROP COLUMN "output",
ADD COLUMN     "lockedById" TEXT;

ALTER TABLE "Job" DROP COLUMN "label",
ADD COLUMN     "queue" TEXT,
ADD COLUMN     "worker" TEXT;
