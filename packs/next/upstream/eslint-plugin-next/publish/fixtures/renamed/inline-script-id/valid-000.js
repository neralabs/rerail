import Script from 'next/script';

      export default function TestWdfq() {
        return (
          <Script jq="test-script">
            {`console.log('Hello world');`}
          </Script>
        )
      }