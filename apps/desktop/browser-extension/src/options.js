const pairingTokenInput = document.getElementById("pairingToken");
const browserFamilyInput = document.getElementById("browserFamily");
const status = document.getElementById("status");
document.getElementById("extensionId").textContent = `Extension ID: ${chrome.runtime.id}`;

chrome.storage.local.get(["pairingToken", "browserFamily"]).then((stored) => {
  pairingTokenInput.value = stored.pairingToken || "";
  browserFamilyInput.value = stored.browserFamily || "chromium";
});

document.getElementById("save").addEventListener("click", () => {
  const pairingToken = pairingTokenInput.value.trim();
  const browserFamily = browserFamilyInput.value;
  status.textContent = "Pairing...";
  chrome.runtime.sendMessage({ channel: "pair", pairingToken, browserFamily }, (response) => {
    if (chrome.runtime.lastError) {
      status.textContent = `Pairing failed: ${chrome.runtime.lastError.message}`;
      return;
    }
    if (response?.ok) {
      status.textContent = pairingToken ? "Paired with Mnema." : "Pairing token cleared.";
    } else {
      status.textContent = `Pairing failed: ${response?.error || "native host unavailable"}`;
    }
  });
});
