
        <template>
          <ul>
            <li
              v-for="{user} in activeUsers"
              :key="user.id"
            >
              {{ user.name }}
            <li>
          </ul>
        </template>
      