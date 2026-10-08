
        <script>
          export default {
            props: ['data'],
            methods: {
              update() {
                return Object.assign({}, this.data, { extra: 'value' })
              }
            }
          }
        </script>
      