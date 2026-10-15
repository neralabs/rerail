
      <script setup>
        const { foo = [] } = defineProps({
          foo: {
            type: Array,
            default: () => {
              return 42
            }
          }
        })
      </script>
      