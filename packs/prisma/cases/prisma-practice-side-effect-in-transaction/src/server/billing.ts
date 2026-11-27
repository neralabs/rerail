import { prisma } from "../lib/db";
import { recordAudit } from "../lib/audit";
import { formatSms, notifyOps, sendSms } from "../lib/notify";
import { sendInvite } from "@shop/emails";

export async function shipOrder(orderId: string, phone: string) {
  await prisma.$transaction(async (tx) => {
    await tx.order.update({ where: { id: orderId }, data: { status: "SHIPPED" } });
    await recordAudit(tx, "ship", orderId);
    const text = formatSms(orderId);
    await sendSms(phone, text); // caught
    await notifyOps(`shipped ${orderId}`); // caught
  });
}

export async function addMember(teamId: string, email: string) {
  return prisma.$transaction(async function addInTx(tx) {
    const team = await tx.team.update({ where: { id: teamId }, data: { members: { create: { email } } } });
    await sendInvite(email, team.name); // caught
    return team;
  });
}
