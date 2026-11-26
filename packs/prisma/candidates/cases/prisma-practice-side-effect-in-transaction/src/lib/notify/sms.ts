export async function sendSms(to: string, body: string) {
  const res = await fetch("https://api.sms.example.com/messages", {
    method: "POST",
    body: JSON.stringify({ to, body }),
  });
  return res.ok;
}

export function formatSms(orderId: string) {
  return `Order ${orderId} shipped`;
}
