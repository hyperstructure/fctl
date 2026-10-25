<script lang="ts">
  let { data } = $props();
</script>

<h1>Settings</h1>

<section>
  <h2>Starter</h2>
  <p>{data.starter.name}, fed with {data.starter.flour}.</p>
</section>
