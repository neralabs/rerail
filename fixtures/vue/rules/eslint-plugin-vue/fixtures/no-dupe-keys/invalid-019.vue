
      <script setup>
        import { Foo } from './Foo.vue';
        import baz from './baz';

        defineProps({
          foo: String,
          bar: String,
          baz: String,
        });

        function foo() {
          const baz = 'baz';
        }
        const bar = () => 'bar';
      </script>
      