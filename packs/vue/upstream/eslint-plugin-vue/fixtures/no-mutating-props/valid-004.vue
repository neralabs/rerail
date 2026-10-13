
        <script>
          export default {
            props: {
              todo: {
                type: Object,
                required: true
              },
              items: {
                type: Array,
                default: []
              }
            },
            methods: {
              openModal() {
                this.$emit('someEvent', this.todo)
                const a = this.items.slice(0).push('something') // no mutation because of `slice(0)`
              }
            }
          }
        </script>
      