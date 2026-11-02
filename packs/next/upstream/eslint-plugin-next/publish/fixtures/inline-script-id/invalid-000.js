import Script from 'next/script';

        export default function TestPage() {
          return (
            <Script>
              {`console.log('Hello world');`}
            </Script>
          )
        }