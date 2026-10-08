
      <template>
        <div v-if      ="((f && e) || d) && c || (b && a)" />
        <div v-else-if ="(a && b) || (c && (d || (e && f)))" />
        <div v-else-if ="(a && b) || (c && (d || (e && f)))" />
      </template>
      