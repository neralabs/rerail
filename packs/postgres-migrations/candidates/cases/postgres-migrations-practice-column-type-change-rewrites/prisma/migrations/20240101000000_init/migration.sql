-- CreateEnum
CREATE TYPE "TaskState" AS ENUM ('OPEN', 'DONE');

-- CreateTable
CREATE TABLE "Event" (
    "id" SERIAL NOT NULL,
    "count" INTEGER NOT NULL,
    "views" INTEGER NOT NULL DEFAULT 0,
    "title" TEXT NOT NULL,
    "name" VARCHAR(100),
    "slug" VARCHAR(40) NOT NULL,
    "price" DECIMAL(10,2) NOT NULL,
    "amount" DECIMAL(10,2) NOT NULL,
    "startsAt" TIMESTAMP(3) NOT NULL,
    "tags" VARCHAR(20)[],

    CONSTRAINT "Event_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "Task" (
    "id" TEXT NOT NULL,
    "state" "TaskState" NOT NULL DEFAULT 'OPEN',

    CONSTRAINT "Task_pkey" PRIMARY KEY ("id")
);

-- AlterTable: the table was created above, so it is still empty
ALTER TABLE "Task" ALTER COLUMN "id" SET DATA TYPE VARCHAR(36);
