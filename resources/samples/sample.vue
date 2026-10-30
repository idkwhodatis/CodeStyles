<template><section class="card"><h1>{{ title }}</h1><button @click="count++">Count: {{ count }}</button></section></template>
<script setup lang="ts">
import {ref} from 'vue';
const title='Vue preview';const count=ref(0);
</script>
<style scoped>.card{padding:16px;color:#345}button{margin-top:8px}</style>
