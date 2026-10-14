
      <script setup>
      const props = defineProps({ count: Number })
      const buildCounter = props.count ? 1 : undefined
      </script>
      