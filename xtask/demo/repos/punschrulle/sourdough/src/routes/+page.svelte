<script lang="ts">
  let { data } = $props();
  const time = (d: Date) => d.toLocaleString("sv-SE", { weekday: "short", hour: "2-digit", minute: "2-digit" });
</script>

{#if data.next}
  <h1>{data.next.recipe}, ready {time(data.next.readyAt)}</h1>
  <ol>
    {#each data.next.steps as step}
      <li><time>{time(step.at)}</time> {step.what}</li>
    {/each}
  </ol>
{:else}
  <h1>No bake planned</h1>
  <a href="/bakes/new">Plan one</a>
{/if}
