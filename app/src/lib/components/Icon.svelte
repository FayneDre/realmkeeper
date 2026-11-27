<script lang="ts" module>
  export type IconName = "play" | "stop" | "plug" | "eye" | "eye-off" | "copy";
</script>

<script lang="ts">
  // Small line icons drawn for the launcher (no third-party set): they take
  // the text colour, so they follow the theme and the button's state.
  let { name }: { name: IconName } = $props();
</script>

<svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
  {#if name === "play"}
    <path d="M8 5.5v13l10-6.5z" class="filled" />
  {:else if name === "stop"}
    <rect x="6.5" y="6.5" width="11" height="11" rx="1.5" class="filled" />
  {:else if name === "plug"}
    <path d="M9 3v4M15 3v4M6.5 7h11v3.5a5.5 5.5 0 0 1-11 0zM12 16v5" />
  {:else if name === "eye"}
    <path d="M2.5 12s3.5-6.5 9.5-6.5 9.5 6.5 9.5 6.5-3.5 6.5-9.5 6.5S2.5 12 2.5 12z" />
    <circle cx="12" cy="12" r="3" />
  {:else if name === "eye-off"}
    <path d="M2.5 12s3.5-6.5 9.5-6.5 9.5 6.5 9.5 6.5-3.5 6.5-9.5 6.5S2.5 12 2.5 12z" />
    <circle cx="12" cy="12" r="3" />
    <path d="M4 20 20 4" />
  {:else if name === "copy"}
    <rect x="8.5" y="8.5" width="11" height="12" rx="1.5" />
    <path d="M15.5 5.5v-1A1.5 1.5 0 0 0 14 3H6A1.5 1.5 0 0 0 4.5 4.5V15A1.5 1.5 0 0 0 6 16.5h1" />
  {/if}
</svg>

<style>
  .icon {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .filled {
    fill: currentColor;
    stroke: none;
  }
</style>
