
      import Script from "next/script";

      export default function Index() {
        return (
          <html lang="en">
            <body className={inter.className}>{children}</body>
            <Script
              src="https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.20/lodash.min.js?a=scriptBeforeInteractive"
              strategy='beforeInteractive'
            />
          </html>
        );
      }