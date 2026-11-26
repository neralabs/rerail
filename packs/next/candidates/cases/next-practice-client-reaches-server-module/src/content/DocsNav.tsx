"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { DOC_SLUGS } from "./DocsPage";

export function DocsNav() {
  const pathname = usePathname();
  return (
    <nav>
      {DOC_SLUGS.map((slug) => (
        <Link key={slug} href={`/docs/${slug}`} aria-current={pathname === `/docs/${slug}` ? "page" : undefined}>
          {slug}
        </Link>
      ))}
    </nav>
  );
}
