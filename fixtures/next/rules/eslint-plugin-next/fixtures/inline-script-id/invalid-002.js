import MyScript from 'next/script';

        export default function TestPage() {
          return (
            <MyScript>
              {`console.log('Hello world');`}
            </MyScript>
          )
        }