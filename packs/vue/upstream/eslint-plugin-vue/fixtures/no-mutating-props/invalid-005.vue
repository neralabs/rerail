
        <script>
          export default {
            props: ['foo', 'bar', 'baz'],
            methods: {
              openModal() {
                this?.foo?.push?.('something')
                ;(this?.bar)?.push?.('something')
                ;(this?.baz?.push)?.('something')
              }
            }
          }
        </script>
      