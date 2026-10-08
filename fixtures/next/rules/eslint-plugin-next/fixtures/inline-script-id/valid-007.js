import Script from 'next/script';
      const spread = { strategy: "lazyOnload" }
      export default function TestPage() {
        return (
          <Script {...spread} id={"test-script"}>
            {`console.log('Hello world');`}
          </Script>
        )
      }