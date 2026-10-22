
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { AppEvent } from './test01'
        const event = ref<AppEvent | undefined>(undefined)
        const result = computed(() => {
          switch (event.value?.type) {
            case 'click': return 'clicked'
            case 'hover': return 'hovered'
            case 'key': return 'pressed'
            case undefined: return 'none'
          }
        })
        </script>