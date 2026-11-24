/** @type {import('next').NextConfig} */
const nextConfig = {
  output: "standalone",
  env: {
    ADMIN_BUILD_ID: process.env.ADMIN_BUILD_ID,
  },
};

export default nextConfig;
