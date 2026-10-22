
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { Status } from './test01'
        function panic(): never { throw new Error() }
        const status = ref<Status>('active')
        const result = computed(() => {
          switch (status.value) {
            case 'active': return 'a'
            case 'inactive': return 'i'
            case 'pending': panic()
          }
        })
        </script>