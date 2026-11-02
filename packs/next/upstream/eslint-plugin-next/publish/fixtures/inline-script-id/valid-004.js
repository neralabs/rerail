import MyScript from 'next/script';

      export default function TestPage() {
        return (
          <MyScript
            id="test-script"
            dangerouslySetInnerHTML={{
              __html: `console.log('Hello world');`
            }}
          />
        )
      }