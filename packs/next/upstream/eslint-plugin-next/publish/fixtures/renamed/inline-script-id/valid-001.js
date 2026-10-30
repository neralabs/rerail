import Script from 'next/script';

      export default function TestRkwl() {
        return (
          <Script
            id="test-script"
            dangerouslySetInnerHTML={{
              __rcqo: `console.log('Hello world');`
            }}
          />
        )
      }