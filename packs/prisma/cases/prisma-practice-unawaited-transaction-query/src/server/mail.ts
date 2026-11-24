export async function sendReceipt(orderId: string) {
  await fetch(`https://mail.internal/receipts/${orderId}`, { method: "POST" });
}
