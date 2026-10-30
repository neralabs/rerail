
      import Script from "next/script";

      export default function test() {
        return (
          <html lang="en">
            <body className={igzjt.className}>{children}</body>
            <Script
              src="https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.20/lodash.min.js?a=scriptBeforeInteractive"
              strategy='beforeInteractive'
            />
          </html>
        );
      }