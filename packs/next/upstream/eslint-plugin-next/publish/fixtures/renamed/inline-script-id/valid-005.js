import Script from 'next/script';

      export default function TestUtpy() {
        return (
          <Script {...{ snkpphqk: "lazyOnload" }} id={"test-script"}>
            {`console.log('Hello world');`}
          </Script>
        )
      }