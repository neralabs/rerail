
      <script setup>
      const props = defineProps({count:Number})
      watch(() => {
        const {count} = props
      })
      watch(() => {
        const count = props.count
      })
      </script>
      