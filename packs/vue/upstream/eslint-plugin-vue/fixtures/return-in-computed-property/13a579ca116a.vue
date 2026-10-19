
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { Status } from './test01'
        const status = ref<Status>('active')
        const result = computed(() => {
          try {
            switch (status.value) {
              case 'active': return 'a'
              case 'inactive': return 'i'
              case 'pending': return 'p'
            }
          } catch (e) {
            return 'err'
          }
        })
        </script>