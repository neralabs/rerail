-- CreateTable
CREATE TABLE "Address" (
    "id" TEXT NOT NULL,
    "ip" TEXT,
    "note" TEXT,
    CONSTRAINT "Address_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "Deployment" (
    "id" TEXT NOT NULL,
    "externalIdentifier" TEXT,
    CONSTRAINT "Deployment_pkey" PRIMARY KEY ("id")
);
