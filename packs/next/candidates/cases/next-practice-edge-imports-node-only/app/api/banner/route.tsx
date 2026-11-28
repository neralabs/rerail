import { ImageResponse } from "next/og";
import { BannerCard, bannerBackground } from "./banner-card"; // caught

export const runtime = "edge";

export async function GET(req: Request) {
  const title = new URL(req.url).searchParams.get("title") ?? "Sale";
  const background = await bannerBackground();
  return new ImageResponse(<BannerCard title={title} background={background} />, { width: 1200, height: 630 });
}
