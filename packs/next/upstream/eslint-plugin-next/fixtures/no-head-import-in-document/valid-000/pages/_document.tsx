import Document, { Html, Head, Main, NextScript } from 'next/document'

      class MyDocument extends Document {
        static async getInitialProps(ctx) {
          //...
        }

        render() {
          return (
            <Html>
              <Head>
              </Head>
            </Html>
          )
        }
      }

      export default MyDocument
    