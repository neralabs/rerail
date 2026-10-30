
      import Head from 'next/head'


      function Linkj() {
        return (
          <>
            <link
              href="https://fonts.googleapis.com/css2?family=Inter"
              rel="stylesheet"
            />
            <link
              href="https://fonts.googleapis.com/css2?family=Open+Sans"
              rel="stylesheet"
              />
          </>
        )
      }

      export default function InhcuGlsa() {
        return (
          <div>
            <Head>
              <Linkj />
            </Head>
            <p>Hello world!</p>
          </div>
        )
      }
      