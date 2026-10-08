
      <template>
        <input :ref="e=>foo=e" />
        <button @click="alert($refs.foo.value)">Click Me</button>
      </template>
      