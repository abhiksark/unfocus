<!-- src/lib/StartAtLoginPrompt.svelte -->

<script lang="ts">
  import type { StartAtLoginState } from "$lib/start-at-login";

  let { state, available, onEnable, onDismiss }: {
    state: StartAtLoginState;
    available: boolean;
    onEnable: () => void;
    onDismiss: () => void;
  } = $props();

  let dialog: HTMLDialogElement;
  let previousFocus: HTMLElement | null = null;
  const shouldOpen = $derived(
    available && state.status?.supported === true &&
      !state.status.enabled && !state.dismissed
  );

  $effect(() => {
    if (shouldOpen && !dialog.open) {
      previousFocus = document.activeElement instanceof HTMLElement
        ? document.activeElement : null;
      dialog.showModal();
    } else if (!shouldOpen && dialog.open) {
      dialog.close();
      if (available) previousFocus?.focus({ preventScroll: true });
    }
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="startup-title"
  aria-describedby="startup-message"
  oncancel={(event) => { event.preventDefault(); onDismiss(); }}
>
  <h2 id="startup-title">Keep your break reminders running</h2>
  <p id="startup-message">
    Start Unfocus when you log in so your reminders return after a restart.
    It will run quietly in the tray.
  </p>
  {#if state.error}
    <p class="error" role="alert">Could not enable start at login: {state.error} Try again.</p>
  {/if}
  <div class="actions" aria-busy={state.pending}>
    <button class="primary" type="button" onclick={onEnable} disabled={state.pending}>
      {state.pending ? "Enabling…" : "Enable start at login"}
    </button>
    <button type="button" onclick={onDismiss}>Not now</button>
  </div>
</dialog>

<style>
  dialog {
    box-sizing: border-box;
    width: min(480px, calc(100% - 2 * var(--s5)));
    max-height: calc(100% - 2 * var(--s5));
    padding: var(--s6);
    border: 1px solid var(--line-2);
    border-radius: var(--r-control);
    background: var(--bg);
    color: var(--ink);
    font-family: var(--sans);
  }
  dialog::backdrop { background: rgb(0 0 0 / 65%); }
  h2 { margin: 0; font-size: 1.5rem; line-height: 1.25; font-weight: 600; }
  p { margin: var(--s4) 0; color: var(--ink-2); line-height: 1.6; }
  .error { color: var(--warn); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--s3); margin-top: var(--s5); }
  button {
    min-height: 44px;
    padding: var(--s3) var(--s4);
    border: 1px solid var(--line-2);
    border-radius: var(--r-button);
    background: var(--bg);
    color: var(--ink);
    font: inherit;
    font-size: 0.875rem;
    cursor: pointer;
  }
  button.primary { background: var(--accent); color: var(--accent-ink); border-color: var(--accent); }
  button.primary:hover { background: var(--accent-hover); }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; }
  button:disabled { opacity: 0.6; cursor: wait; }
</style>
