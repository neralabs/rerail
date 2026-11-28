import { PrismaClient } from "@prisma/client";

export class AuditService {
  private readonly prisma = new PrismaClient();
  private readonly replica: PrismaClient;

  constructor() {
    this.replica = new PrismaClient({ datasourceUrl: process.env.REPLICA_URL });
  }

  async record(event: string) {
    await this.prisma.auditEvent.create({ data: { event } });
  }
}
