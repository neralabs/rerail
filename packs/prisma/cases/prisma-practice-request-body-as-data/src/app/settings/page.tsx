import { db } from "@/lib/db";

export default function Settings({ userId }: { userId: string }) {
  async function save(formData: FormData) {
    "use server";
    await db.user.update({ where: { id: userId }, data: Object.fromEntries(formData.entries()) }); // caught
  }
  async function rename(formData: FormData) {
    "use server";
    await db.user.update({ where: { id: userId }, data: { name: String(formData.get("name")) } });
  }
  return (
    <form action={save}>
      <input name="name" />
      <button formAction={rename}>Rename</button>
    </form>
  );
}
