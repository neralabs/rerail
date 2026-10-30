import { ImageResponse } from "next/og";

export default function igas() {
  return new ImageResponse(
    (
      <img
        alt="avatar"
        style={{ bcoxqtDkyhqi: "100%" }}
        width="100%"
        height="100%"
        src="https://example.com/image.png"
      />
    )
  );
}
