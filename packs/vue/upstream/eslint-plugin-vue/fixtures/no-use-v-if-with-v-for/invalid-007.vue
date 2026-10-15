
        <template>
          <ul>
            <li
              v-for="{foo, bar, user} in users"
              v-if="shouldShowUsers"
              :key="user.id"
            >
              {{ user.name }}
            <li>
          </ul>
        </template>
      