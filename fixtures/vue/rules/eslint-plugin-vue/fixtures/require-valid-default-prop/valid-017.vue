
      <script setup>
        const { foo = 'abc' } = defineProps({
          foo: {
            type: String,
          }
        })
      </script>
      