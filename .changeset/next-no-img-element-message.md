---
"@neralabs/rerail-pack-next": patch
---

Next pack: `next-no-img-element`'s message now says what switching to `next/image` needs (`width` and `height`, or `fill`, unless `src` is a static import; a matching `images.remotePatterns` entry for a remote `src`) and that `// eslint-disable-next-line @next/next/no-img-element -- <reason>` keeps `<img>`, for example for images from arbitrary hosts. What it reports is unchanged.
