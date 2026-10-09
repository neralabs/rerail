import MyScript from 'next/script';

        export default function TestPage() {
          return (
            <MyScript
              dangerouslySetInnerHTML={{
                __html: `console.log('Hello world');`
              }}
            />
          )
        }