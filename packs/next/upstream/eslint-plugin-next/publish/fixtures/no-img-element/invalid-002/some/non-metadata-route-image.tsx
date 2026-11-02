import { ImageResponse } from "next/og";

export default function Image() {
return new ImageResponse(
  (
    <img
      alt="avatar"
      style={{ borderRadius: "100%" }}
      width="100%"
      height="100%"
      src="https://example.com/image.png"
    />
  )
);
}
