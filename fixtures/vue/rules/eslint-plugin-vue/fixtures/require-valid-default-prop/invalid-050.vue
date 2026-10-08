
      <script setup>
        const { foo = (()=>[]) } = defineProps({
          foo: {
            type: Array,
          }
        })
      </script>
      