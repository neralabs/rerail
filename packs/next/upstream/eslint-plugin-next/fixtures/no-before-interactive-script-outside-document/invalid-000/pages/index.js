
      import Head from "next/head";
      import Script from "next/script";

      export default function Index() {
        return (
          <Script
            id="scriptBeforeInteractive"
            src="https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.20/lodash.min.js?a=scriptBeforeInteractive"
            strategy="beforeInteractive"
          ></Script>
        );
      }