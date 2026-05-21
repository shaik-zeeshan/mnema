const TEXT_LIKE_TYPES = new Set([
  "",
  "email",
  "number",
  "search",
  "tel",
  "text",
  "url"
]);
const CREDENTIAL_AUTOCOMPLETE = new Set([
  "username",
  "current-password",
  "new-password",
  "one-time-code"
]);

let sequence = 0;
let lastStateKey = "";

function isTextLikeControl(element) {
  if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement)) return false;
  if (element instanceof HTMLTextAreaElement) return true;
  return TEXT_LIKE_TYPES.has((element.getAttribute("type") || "").toLowerCase());
}

function formHasPasswordControl(form) {
  return Boolean(form?.querySelector?.('input[type="password"]'));
}

function nearestCredentialGroupHasPasswordControl(element) {
  let current = element.parentElement;
  for (let depth = 0; current && depth < 4; depth += 1) {
    if (current.querySelector?.('input[type="password"]')) return true;
    current = current.parentElement;
  }
  return false;
}

function autocompleteHasCredentialToken(element) {
  const tokens = (element.getAttribute("autocomplete") || "")
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  return tokens.some((token) => CREDENTIAL_AUTOCOMPLETE.has(token));
}

function detectSecureEntry() {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement)) {
    return { state: "clear", reason: "no_focused_credential_control" };
  }
  if (active instanceof HTMLInputElement && (active.getAttribute("type") || "").toLowerCase() === "password") {
    return { state: "active", reason: "focused_password_control" };
  }
  if (isTextLikeControl(active)) {
    const hasCredentialStructure = formHasPasswordControl(active.form) || nearestCredentialGroupHasPasswordControl(active);
    if (hasCredentialStructure) {
      if (autocompleteHasCredentialToken(active)) {
        return { state: "active", reason: "focused_autocomplete_credential_control" };
      }
      return { state: "active", reason: "focused_related_credential_control" };
    }
  }
  return { state: "clear", reason: "no_focused_credential_control" };
}

function emitSecureEntry(force = false) {
  const result = detectSecureEntry();
  const key = `${result.state}:${result.reason}`;
  if (!force && key === lastStateKey) return;
  lastStateKey = key;
  sequence += 1;
  chrome.runtime.sendMessage({
    channel: "secureEntry",
    signal: {
      version: 1,
      kind: "browser_secure_entry_signal",
      state: result.state,
      reason: result.reason,
      observedAtUnixMs: Date.now(),
      sequence
    }
  });
}

document.addEventListener("focusin", () => emitSecureEntry(true), true);
document.addEventListener("focusout", () => setTimeout(() => emitSecureEntry(true), 0), true);
setInterval(() => emitSecureEntry(true), 1000);
