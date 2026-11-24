-- Step 1: a CHECK that no row is NULL, added NOT VALID (no scan).
ALTER TABLE "Account" ADD CONSTRAINT "Account_provider_not_null" CHECK ("provider" IS NOT NULL) NOT VALID;

-- Added NOT VALID and never validated: it proves nothing to SET NOT NULL.
ALTER TABLE "Session" ADD CONSTRAINT "Session_userAgent_not_null" CHECK ("userAgent" IS NOT NULL) NOT VALID;
