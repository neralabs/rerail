"use client";

import { SUPPORT_EMAIL } from "@/lib/mailer";

export function ContactForm() {
  return (
    <form action="/api/contact" method="post">
      <textarea name="body" required />
      <button type="submit">Send</button>
      <p>
        Or write to <a href={`mailto:${SUPPORT_EMAIL}`}>{SUPPORT_EMAIL}</a>
      </p>
    </form>
  );
}
