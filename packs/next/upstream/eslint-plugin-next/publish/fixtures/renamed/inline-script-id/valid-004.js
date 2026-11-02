import MmUvllfb from 'next/script';

      export default function TestTunv() {
        return (
          <MmUvllfb
            id="test-script"
            dangerouslySetInnerHTML={{
              __html: `console.log('Hello world');`
            }}
          />
        )
      }