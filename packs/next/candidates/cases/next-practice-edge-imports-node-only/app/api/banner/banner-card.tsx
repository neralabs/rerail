import sharp from "sharp";

export async function bannerBackground() {
  const png = await sharp({ create: { width: 1200, height: 630, channels: 3, background: "#0f172a" } }).png().toBuffer();
  return `data:image/png;base64,${png.toString("base64")}`;
}

export function BannerCard({ title, background }: { title: string; background: string }) {
  return (
    <div style={{ display: "flex", backgroundImage: `url(${background})`, fontSize: 72, color: "white" }}>{title}</div>
  );
}
