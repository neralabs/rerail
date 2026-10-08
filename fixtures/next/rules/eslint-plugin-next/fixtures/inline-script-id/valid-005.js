import Script from 'next/script';

      export default function TestPage() {
        return (
          <Script {...{ strategy: "lazyOnload" }} id={"test-script"}>
            {`console.log('Hello world');`}
          </Script>
        )
      }