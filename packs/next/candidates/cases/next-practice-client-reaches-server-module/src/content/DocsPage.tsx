import { readFile } from "fs/promises"; // caught
import path from "path";

export const DOC_SLUGS = ["getting-started", "billing", "api"] as const;

export async function DocsPage({ slug }: { slug: (typeof DOC_SLUGS)[number] }) {
  const md = await readFile(path.join(process.cwd(), "content", `${slug}.md`), "utf8");
  return <article dangerouslySetInnerHTML={{ __html: md }} />;
}
