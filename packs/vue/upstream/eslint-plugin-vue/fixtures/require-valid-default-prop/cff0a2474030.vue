
      <script setup lang="ts">
      import {Props2 as Props} from './test01'
      withDefaults(defineProps<Props>(), {
        a: 42,
        b: 's',
        c: {},
        d: [],
        e: [42],
        f: {},
        g: { foo: 'foo' },
        h: ['foo', 'bar'],
        i: ['foo', 'bar'],
      })
      </script>