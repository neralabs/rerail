import Document, { Html, Head } from "next/document";

     class MyDocument extends Document {
      render() {
        return (
          <Html>
            <Head>
              <link
                href="https://fonts.googleapis.com/css?family=Krona+One&display=swap"
                rel="stylesheet"
                crossOrigin=""
              />
            </Head>
          </Html>
        );
      }
     }

     export default MyDocument;
    