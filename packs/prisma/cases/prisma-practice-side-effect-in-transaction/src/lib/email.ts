import { Resend } from "resend";

const resend = new Resend(process.env.RESEND_API_KEY);

export function renderReceipt(orderId: string, total: number) {
  return `<p>Order ${orderId}: ${total}</p>`;
}

export async function sendReceipt(to: string, orderId: string, total: number) {
  await resend.emails.send({
    from: "shop@example.com",
    to,
    subject: "Your receipt",
    html: renderReceipt(orderId, total),
  });
}

export const notifyWarehouse = async (orderId: string) => {
  await fetch(`https://warehouse.example.com/orders/${orderId}`, { method: "POST" });
};
