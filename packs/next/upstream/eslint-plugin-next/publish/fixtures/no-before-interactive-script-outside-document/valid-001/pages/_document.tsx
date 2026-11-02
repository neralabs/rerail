
      import Document, { Html, Main, NextScript } from 'next/document'
      import ScriptComponent from 'next/script'

      class MyDocument extends Document {
        render() {
          return (
            <Html>
              <Head>
                <meta charSet="utf-8" />
              </Head>
              <body>
                <Main />
                <NextScript />
                <ScriptComponent
                  id="scriptBeforeInteractive"
                  src="https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.20/lodash.min.js?a=scriptBeforeInteractive"
                  strategy="beforeInteractive"
                ></ScriptComponent>
              </body>
            </Html>
          )
        }
      }

      export default MyDocument
      