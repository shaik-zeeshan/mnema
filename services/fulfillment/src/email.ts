// License fulfillment email. Inline styles only — email clients strip <style>
// blocks and don't do external CSS. Palette + type mirror the marketing site
// (apps/web/src/styles/tokens.css); custom fonts can't load in mail, so we fall
// back to system-ui / ui-monospace.

const SITE = "https://mnema.day";
// Hosted so it renders without a data-URI (Gmail proxies remote images; many
// clients drop inline base64). Served from apps/web/public/icon.png at the root.
const LOGO_URL = `${SITE}/icon.png`;

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// Activation page (`/activate#key=…`). The key rides in the URL *hash* so it
// never reaches server/CDN logs; the page reads it client-side and offers a
// Copy button + the `mnema://license/activate` deep link. Key is standard base64
// (`+ / =`) joined by `.`, so it MUST be percent-encoded.
export function activationUrl(key: string): string {
  return `${SITE}/activate#key=${encodeURIComponent(key)}`;
}

export function licenseEmail(key: string): { subject: string; text: string; html: string } {
  const subject = "Your Mnema license key";
  const activate = activationUrl(key);

  const text =
    `Thanks for buying Mnema.\n\n` +
    `Activate here (opens the app, or copy your key):\n${activate}\n\n` +
    `Or paste this key into Settings → License:\n\n${key}\n\n` +
    `Keep this email — it's your proof of purchase.\n`;

  const safeKey = escapeHtml(key);
  const safeLink = escapeHtml(activate);

  // Table layout + inline styles: the only thing that renders consistently across
  // Gmail / Outlook / Apple Mail. The deep-link button is the happy path; the key
  // box below is the fallback for clients that strip custom-scheme hrefs.
  const html = `<!doctype html>
<html>
  <body style="margin:0;padding:0;background:#0c0c0e;">
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#0c0c0e;">
      <tr>
        <td align="center" style="padding:40px 16px;">
          <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:480px;background:#14141a;border:1px solid #1e1e2e;border-radius:14px;overflow:hidden;">
            <tr>
              <td style="padding:32px 32px 0;">
                <img src="${LOGO_URL}" width="44" height="44" alt="Mnema" style="display:block;border-radius:10px;" />
              </td>
            </tr>
            <tr>
              <td style="padding:20px 32px 8px;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;color:#e2e2e8;font-size:22px;font-weight:600;">
                Thanks for buying Mnema.
              </td>
            </tr>
            <tr>
              <td style="padding:0 32px 24px;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;color:#8a8aaa;font-size:14px;line-height:1.6;">
                Activate opens Mnema and applies your license — or copy your key there to paste manually.
              </td>
            </tr>
            <tr>
              <td style="padding:0 32px 28px;">
                <a href="${safeLink}" style="display:inline-block;background:#3dffa0;color:#0c0c0e;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-size:15px;font-weight:600;text-decoration:none;padding:13px 28px;border-radius:10px;">
                  Activate Mnema
                </a>
              </td>
            </tr>
            <tr>
              <td style="padding:0 32px 10px;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;color:#8a8aaa;font-size:13px;line-height:1.6;">
                Button didn't open the app? Paste this key into <strong style="color:#e2e2e8;">Settings → License</strong>:
              </td>
            </tr>
            <tr>
              <td style="padding:0 32px 28px;">
                <div style="font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;color:#3dffa0;font-size:12px;line-height:1.5;background:#0c0c0e;border:1px solid #1e1e2e;border-radius:8px;padding:14px;word-break:break-all;">
                  ${safeKey}
                </div>
              </td>
            </tr>
            <tr>
              <td style="padding:20px 32px;border-top:1px solid #1e1e2e;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;color:#8a8aaa;font-size:12px;line-height:1.6;">
                Keep this email — it's your proof of purchase.
              </td>
            </tr>
          </table>
        </td>
      </tr>
    </table>
  </body>
</html>`;

  return { subject, text, html };
}
