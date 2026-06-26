<script lang="ts">
  // A plain text input. `value` is a RAW STRING so it flows upward unchanged
  // into raw draft fields (e.g. customWidthRaw / customHeightRaw) that parse and
  // validate with their own integer regex. Empty string = unset. Ships its own
  // styles (not the `.settings-shell .text-input` rules) so it renders the same
  // inside settings and onboarding.
  let {
    value = $bindable(""),
    inputmode = "text",
    placeholder,
    disabled = false,
    invalid = false,
    id,
    ariaLabel,
    errorId,
  }: {
    value?: string;
    inputmode?: "text" | "numeric" | "decimal" | "tel" | "email" | "url" | "search" | "none";
    placeholder?: string;
    disabled?: boolean;
    invalid?: boolean;
    id?: string;
    ariaLabel?: string;
    // Id of the element holding the validation message; wired to
    // aria-describedby/aria-errormessage while `invalid` so AT announces the
    // reason, not just that the field is invalid.
    errorId?: string;
  } = $props();
</script>

<input
  {id}
  type="text"
  {inputmode}
  {placeholder}
  {disabled}
  class="input"
  class:input--invalid={invalid}
  bind:value
  aria-label={ariaLabel}
  aria-invalid={invalid}
  aria-describedby={invalid && errorId ? errorId : undefined}
  aria-errormessage={invalid && errorId ? errorId : undefined}
  autocomplete="off"
/>

<style>
  .input {
    width: 100%;
    min-width: 0;
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    background: var(--app-surface);
    color: var(--app-text);
    font: inherit;
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 12px;
    outline: none;
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.25);
    transition: border-color 0.12s, box-shadow 0.12s, background 0.12s;
  }

  .input::placeholder {
    color: var(--app-text-faint);
  }

  .input:focus {
    border-color: var(--app-accent);
    background: var(--app-surface-raised);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.25), var(--app-ring);
  }

  .input--invalid {
    border-color: var(--app-danger);
  }

  .input--invalid:focus {
    border-color: var(--app-danger);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.25),
      0 0 0 3px color-mix(in srgb, var(--app-danger) 30%, transparent);
  }

  .input:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }
</style>
