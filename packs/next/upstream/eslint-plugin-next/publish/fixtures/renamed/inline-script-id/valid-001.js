import Script from 'next/script';

      export default function TestRkwl() {
        return (
          <Script
            id="test-script"
            dangerouslySetInnerHTML={{
              __html: `console.log('Hello world');`
            }}
          />
        )
      }