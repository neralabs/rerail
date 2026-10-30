import Script from 'next/script';

      export default function TestPage() {
        return (
          <Script
            id="test-script"
            dangerouslySetInnerHTML={{
              __html: `console.log('Hello world');`
            }}
          />
        )
      }