-- Step 2, deployed separately: validate under SHARE UPDATE EXCLUSIVE (reads and writes go on).
ALTER TABLE "Account" VALIDATE CONSTRAINT "Account_provider_not_null";
