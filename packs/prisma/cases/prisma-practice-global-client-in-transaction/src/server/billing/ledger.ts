import { PrismaClient } from "@prisma/client";

export class LedgerService {
  constructor(private readonly prisma: PrismaClient) {}

  async transfer(from: string, to: string, amount: number) {
    await this.prisma.$transaction(async (tx) => {
      await tx.account.update({ where: { id: from }, data: { balance: { decrement: amount } } });
      await this.prisma.account.update({ where: { id: to }, data: { balance: { increment: amount } } }); // caught
    });
  }

  async report() {
    const accounts = await this.prisma.account.findMany();
    await this.prisma.$transaction(async (tx) => {
      await tx.report.create({ data: { total: accounts.length } });
    });
  }
}
