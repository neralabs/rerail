import got from "got";
import OpenAI from "openai";
import { prisma } from "@/lib/db";
import { stripe } from "@/lib/stripe";

const openai = new OpenAI();

export async function summarise(postId: string) {
  await prisma.$transaction(async (tx) => {
    const post = await tx.post.findUniqueOrThrow({ where: { id: postId } });
    const res = await openai.chat.completions.create({ model: "gpt-4o-mini", messages: [] }); // caught
    await got.post("https://hooks.example.com/summary", { json: { postId } }); // caught
    await tx.post.update({ where: { id: post.id }, data: { summary: res.choices[0]?.message.content ?? "" } });
  });
}

export async function testSignature(payload: string) {
  await prisma.$transaction(async (tx) => {
    // Builds a signature header locally, as constructEvent checks one.
    const header = stripe.webhooks.generateTestHeaderString({ payload, secret: "whsec_test" });
    await tx.stripeEvent.create({ data: { id: header, type: "test" } });
  });
}
