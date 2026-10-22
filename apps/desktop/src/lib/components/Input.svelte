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
  class="mx-input"
  class:num={inputmode === "numeric" || inputmode === "decimal"}
  bind:value
  aria-label={ariaLabel}
  aria-invalid={invalid}
  aria-describedby={invalid && errorId ? errorId : undefined}
  aria-errormessage={invalid && errorId ? errorId : undefined}
  autocomplete="off"
/>

<style>
  /* Look lives in kit.css (.mx-input; aria-invalid = danger border). Call sites
     size it by its container. */
  .mx-input {
    width: 100%;
    min-width: 0;
  }
</style>
