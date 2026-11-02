import Script from 'next/script';

      export default function TestUwdf() {
        return (
          <Script id="test-script">
            {`console.log('Hello world');`}
          </Script>
        )
      }