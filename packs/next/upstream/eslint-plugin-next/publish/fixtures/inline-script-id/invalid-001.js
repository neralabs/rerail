import Script from 'next/script';

        export default function TestPage() {
          return (
            <Script
              dangerouslySetInnerHTML={{
                __html: `console.log('Hello world');`
              }}
            />
          )
        }