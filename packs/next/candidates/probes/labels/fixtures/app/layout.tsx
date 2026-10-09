import type { Metadata, Viewport } from "next";

export const metadata: Metadata = { title: "Probe" };
export const viewport: Viewport = { themeColor: "black" };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
