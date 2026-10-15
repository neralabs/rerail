
      <script setup>
      const props = defineProps({count:Number})
      watch(() => {
        ({ count } = props)
      })
      watch(() => {
        count = props.count
      })
      </script>
      