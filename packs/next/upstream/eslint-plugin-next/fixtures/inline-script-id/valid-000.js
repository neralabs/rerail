import Script from 'next/script';

      export default function TestPage() {
        return (
          <Script id="test-script">
            {`console.log('Hello world');`}
          </Script>
        )
      }