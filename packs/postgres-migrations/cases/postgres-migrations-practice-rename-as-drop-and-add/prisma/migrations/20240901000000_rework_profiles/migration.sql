-- AlterTable: profileBio is renamed (reported); profileURL moves elsewhere and bio is not its new name.
ALTER TABLE "UserProfile" DROP COLUMN "profileBio", -- caught
DROP COLUMN "profileURL",
ADD COLUMN     "bio" TEXT;
