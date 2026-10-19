
        <script setup lang="ts" generic="T extends Status">
        import { computed } from 'vue'
        import { Status } from './test01'
        const props = defineProps<{ value: T }>()
        const result = computed(() => {
          switch (props.value) {
            case 'active': return 1
            case 'inactive': return 2
          }
        })
        </script>