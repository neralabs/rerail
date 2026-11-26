import { cookies } from "next/headers";
import { prisma } from "@/lib/db";

// A cached component may define a Server Action: the action runs later, in its own request.
export async function NewsletterForm() {
  "use cache";
  async function subscribe(formData: FormData) {
    "use server";
    const store = await cookies();
    await prisma.subscriber.create({ data: { email: String(formData.get("email")), ref: store.get("ref")?.value } });
  }
  return (
    <form action={subscribe}>
      <input name="email" />
    </form>
  );
}
