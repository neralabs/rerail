-- AlterTable
ALTER TABLE "Event" ALTER COLUMN "count" SET DATA TYPE BIGINT; -- caught
ALTER TABLE "Event" ALTER COLUMN "title" SET DATA TYPE VARCHAR(80), -- caught
ALTER COLUMN "name" SET DATA TYPE VARCHAR(50); -- caught
ALTER TABLE "Event" ALTER COLUMN "amount" SET DATA TYPE DECIMAL(10,4); -- caught

-- AlterEnum
BEGIN;
CREATE TYPE "TaskState_new" AS ENUM ('OPEN', 'IN_PROGRESS', 'DONE');
ALTER TABLE "Task" ALTER COLUMN "state" DROP DEFAULT;
ALTER TABLE "Task" ALTER COLUMN "state" TYPE "TaskState_new" USING ("state"::text::"TaskState_new"); -- caught
ALTER TYPE "TaskState" RENAME TO "TaskState_old";
ALTER TYPE "TaskState_new" RENAME TO "TaskState";
DROP TYPE "TaskState_old";
ALTER TABLE "Task" ALTER COLUMN "state" SET DEFAULT 'OPEN';
COMMIT;
