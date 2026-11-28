import { transporter } from "./transport";

export async function sendInvite(to: string, teamName: string) {
  await transporter.sendMail({ to, subject: `Join ${teamName}`, text: "You are invited." });
}
