import Head from "next/head";

     export default Test = () => {
      return (
        <Head>
          <link href={test} rel="test" />
          <link
            href={process.env.NEXT_PUBLIC_CANONICAL_URL}
            rel="canonical"
          />
          <link
            href={new URL("../public/favicon.ico", import.meta.url).toString()}
            rel="icon"
          />
          <link
            href="https://fonts.googleapis.com/css2?family=Krona+One&display=optional"
            rel="stylesheet"
          />
        </Head>
      );
     };
    