<script setup lang="ts">
const { data: cart } = await useFetch('/api/cart');
</script>

<template>
  <p>{{ cart }}</p>
</template>
