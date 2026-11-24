"use client";

import sharp from "sharp"; // caught
import { useState } from "react";

export function AvatarUpload({ onReady }: { onReady: (png: Uint8Array) => void }) {
  const [busy, setBusy] = useState(false);
  return (
    <input
      type="file"
      accept="image/*"
      disabled={busy}
      onChange={async (e) => {
        const file = e.target.files?.[0];
        if (!file) return;
        setBusy(true);
        const resized = await sharp(Buffer.from(await file.arrayBuffer())).resize(256, 256).png().toBuffer();
        onReady(new Uint8Array(resized));
        setBusy(false);
      }}
    />
  );
}
