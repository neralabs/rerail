import nodemailer from "nodemailer"; // cross-lane: missed until the engine joins .ts and .tsx

const transport = nodemailer.createTransport(process.env.SMTP_URL);

export const SUPPORT_EMAIL = "support@shop.example";

export async function sendSupportMail(from: string, body: string) {
  await transport.sendMail({ from, to: SUPPORT_EMAIL, subject: "Support request", text: body });
}
