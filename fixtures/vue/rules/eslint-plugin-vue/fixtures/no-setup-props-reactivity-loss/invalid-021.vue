
      <script setup>
      const props = defineProps({ count: Number })
      const buildCounter = (count) => ({ count })

      buildCounter(props.count)
      </script>
      