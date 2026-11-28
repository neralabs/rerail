import OpenAI from "openai";

const openai = new OpenAI({ apiKey: process.env["NEXT_PUBLIC_OPENAI_API_KEY"] }); // caught

export async function POST(request: Request) {
  const { prompt } = await request.json();
  const completion = await openai.chat.completions.create({
    model: "gpt-4o-mini",
    messages: [{ role: "user", content: prompt }],
  });
  return Response.json(completion.choices[0].message);
}
