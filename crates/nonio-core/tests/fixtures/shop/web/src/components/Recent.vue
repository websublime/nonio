<script setup lang="ts">
import { request } from '../api/request';

async function recent() {
  return request('/orders');
}
</script>

<template>
  <button @click="recent">Recent</button>
</template>
