import Document, { Html, Head, Main, NextScript } from 'next/document'

        export class MyDocument extends Document {
          static async getInitialProps(ctx) {
            const initialProps = await Document.getInitialProps(ctx)
            return { ...initialProps }
          }

          render() {
            return (
              <Html>
                <Head />
                <style>{"                  body{                    color:red;                  }                "}</style>
                <style {...{nonce: '123' }}></style>
                <body>
                  <Main />
                  <NextScript />
                </body>
              </Html>
            )
          }
        }