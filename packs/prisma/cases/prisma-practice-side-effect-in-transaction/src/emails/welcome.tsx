import { Resend } from "resend";

const resend = new Resend(process.env.RESEND_API_KEY);

function Welcome({ name }: { name: string }) {
  return <p>Welcome, {name}!</p>;
}

export async function sendWelcome(to: string, name: string) {
  await resend.emails.send({ from: "hi@example.com", to, subject: "Welcome", react: <Welcome name={name} /> });
}
