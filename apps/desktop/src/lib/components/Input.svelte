<script lang="ts">
  // A plain text input. `value` is a RAW STRING so it flows upward unchanged
  // into raw draft fields (e.g. customWidthRaw / customHeightRaw) that parse and
  // validate with their own integer regex. Empty string = unset. Renders off
  // the shared `.input` primitive (not the `.settings-shell .text-input` rules)
  // so it looks the same inside settings and onboarding.
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
  /* `.input` is the shared primitive (system.css §6, routes/+layout.svelte).
     Only what makes THIS an in-row form field lives here. */
  .input {
    width: 100%;
    min-width: 0;
  }

  .input::placeholder {
    /* Format hints must clear AA contrast; --app-text-faint is decoration-only
       and falls below it. Match Select/Combobox placeholder text. */
    color: var(--app-text-subtle);
  }

  /* The base `.input` is borderless in this direction — its rim is an inset
     shadow, so the invalid/danger state has to speak in the same language. */
  .input--invalid {
    box-shadow: inset 0 0 0 var(--hairline) var(--app-danger);
  }

  .input--invalid:focus {
    box-shadow: inset 0 0 0 var(--hairline) var(--app-danger),
      var(--ring-danger);
  }

  .input:disabled {
    opacity: var(--opacity-disabled);
    cursor: not-allowed;
  }
</style>
