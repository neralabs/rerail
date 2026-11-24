-- @param {String} $1:tier
SELECT "id", "provider" FROM "Account" WHERE "provider" = $1
