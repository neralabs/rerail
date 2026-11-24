-- AlterTable: a queue name replaced by a reference to the new Queue table; the names are no ids.
ALTER TABLE "Run" DROP COLUMN "queueName",
ADD COLUMN     "queueId" TEXT;
