-- Added nullable, backfilled, then made NOT NULL in one file: reported once per statement (the message leads with
-- `ADD COLUMN … NOT NULL DEFAULT 0`, since the backfill is a constant)
ALTER TABLE "Post" ADD COLUMN "views" INTEGER,
ADD COLUMN "likes" INTEGER;
UPDATE "Post" SET "views" = 0, "likes" = 0;
ALTER TABLE "Post" ALTER COLUMN "views" SET NOT NULL, -- caught
ALTER COLUMN "likes" SET NOT NULL;
