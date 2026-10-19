
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { Status } from './test01'
        const a = ref<Status>('active')
        const b = ref<boolean>(true)
        const result = computed(() => {
          switch (a.value) {
            case 'active':
              switch (b.value) {
                case true: return 'a-true'
                case false:
                  switch (b.value) {
                    case true: return 'never'
                    case false: return 'a-false-false'
                  }
              }
            case 'inactive': return 'i'
            case 'pending': return 'p'
          }
        })
        </script>