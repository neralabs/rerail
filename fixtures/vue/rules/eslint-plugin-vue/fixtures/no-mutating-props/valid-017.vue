
        <script>
        // not <script setup>
        const {value} = defineProps({
          value: Object,
        })
        value.value++
        </script>
        <script setup>
        value.value++
        </script>
      