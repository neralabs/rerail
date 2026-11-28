const { PrismaClient } = require("@prisma/client");

module.exports = async function handler(req, res) {
  const prisma = new PrismaClient(); // caught
  res.json(await prisma.invoice.findMany());
};
