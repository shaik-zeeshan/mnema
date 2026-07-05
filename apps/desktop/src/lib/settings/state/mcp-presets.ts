// MCP connector preset catalog (Plan: MCP Connector Preset Picker, Slice 1).
//
// Every entry here was VERIFIED against the vendor's current docs (and, for
// hosted endpoints, a live probe) before inclusion — see the per-preset comment
// blocks. The v1 rule (plan "Implementation Decisions"): hosted presets must
// accept a pasted static token via `Authorization: Bearer <token>`, because
// that is the ONLY way Mnema delivers an HTTP connector's secret
// (`ask_ai/mcp/transport.rs`). OAuth-only services are dropped, not worked
// around.
//
// Dropped from the mockup catalog after verification (2026-07-05):
//   • Sentry   — hosted mcp.sentry.dev is OAuth-only; static-token auth is an
//                open upstream request (getsentry/sentry-mcp#833).
//   • Context7 — key is passed via a custom `CONTEXT7_API_KEY` header, not
//                `Authorization: Bearer`; a live probe showed a bogus Bearer
//                token is silently accepted (ignored), so a pasted key would
//                be a silent no-op.
//   • Notion (hosted) — mcp.notion.com is OAuth-only; replaced by Notion's
//                official LOCAL server, which takes a static integration token.
//   • Postgres — `@modelcontextprotocol/server-postgres` is deprecated on npm
//                ("Package no longer supported"; reference server archived).
//
// Keep this file free of `$state`/`$derived`/`$effect` and of Tauri `invoke`:
// plain data + pure functions so `bun test` can exercise it directly
// (mcp-tool-curation.ts / ai-providers.ts precedent).

import type { McpServerConfig } from "$lib/types";
import { newMcpServerId } from "./ai-providers";

export interface McpPreset {
	/** Catalog identity for the tile (NOT the draft id — that is slugged per add). */
	id: string;
	label: string;
	/** One-liner on the step-1 grid tile. */
	tagline: string;
	/** Step-2 capability sentence ("Chat can …"). */
	lede: string;
	kind: "hosted" | "local";
	/** hosted: the streamable-HTTP MCP endpoint. */
	url?: string;
	/** local: the child-process command (spawned directly, no shell). */
	command?: string;
	/** local: arguments passed to `command`. */
	args?: string[];
	/** Label for the ONE token field; absent = the preset needs no secret. */
	secretLabel?: string;
	/** local-with-secret: the env var the keychain secret is delivered as. */
	secretEnvName?: string;
	/** "Create one →" token-creation page; present iff `secretLabel` is. */
	helpUrl?: string;
	/** Inline monochrome brand mark (currentColor) for the tile icon. */
	brandSvg: string;
}

// Brand marks lifted from docs/mockups/mcp-connectors/a-modal-grid.html
// (monochrome, currentColor). Filesystem uses the generic folder glyph.
const SVG_GITHUB = `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"/></svg>`;
const SVG_LINEAR = `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M2.886 4.18A11.982 11.982 0 0 1 11.99 0C18.624 0 24 5.376 24 12.009c0 3.64-1.62 6.903-4.18 9.105L2.887 4.18ZM1.817 5.626l16.556 16.556c-.524.33-1.075.62-1.65.866L.951 7.277c.247-.575.537-1.126.866-1.65ZM.322 9.163l14.515 14.515c-.71.172-1.443.282-2.195.322L0 11.358a12 12 0 0 1 .322-2.195Zm-.17 4.862 9.823 9.824a12.02 12.02 0 0 1-9.824-9.824Z"/></svg>`;
const SVG_STRIPE = `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M13.976 9.15c-2.172-.806-3.356-1.426-3.356-2.409 0-.831.683-1.305 1.901-1.305 2.227 0 4.515.858 6.09 1.631l.89-5.494C18.252.975 15.697 0 12.165 0 9.667 0 7.589.654 6.104 1.872 4.56 3.147 3.757 4.992 3.757 7.218c0 4.039 2.467 5.76 6.476 7.219 2.585.92 3.445 1.574 3.445 2.583 0 .98-.84 1.545-2.354 1.545-1.875 0-4.965-.921-6.99-2.109l-.9 5.555C5.175 22.99 8.385 24 11.714 24c2.641 0 4.843-.624 6.328-1.813 1.664-1.305 2.525-3.236 2.525-5.732 0-4.128-2.524-5.851-6.594-7.305h.003z"/></svg>`;
const SVG_NOTION = `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M4.459 4.208c.746.606 1.026.56 2.428.466l13.215-.793c.28 0 .047-.28-.046-.326L17.86 1.968c-.42-.326-.981-.7-2.055-.607L3.01 2.295c-.466.046-.56.28-.374.466zm.793 3.08v13.904c0 .747.373 1.027 1.214.98l14.523-.84c.841-.046.935-.56.935-1.167V6.354c0-.606-.233-.933-.748-.887l-15.177.887c-.56.047-.747.327-.747.933zm14.337.745c.093.42 0 .84-.42.888l-.7.14v10.264c-.608.327-1.168.514-1.635.514-.748 0-.935-.234-1.495-.933l-4.577-7.186v6.952L12.21 19s0 .84-1.168.84l-3.222.186c-.093-.186 0-.653.327-.746l.84-.233V9.854L7.822 9.76c-.094-.42.14-1.026.793-1.073l3.456-.233 4.764 7.279v-6.44l-1.215-.139c-.093-.514.28-.887.747-.933zM1.936 1.035l13.31-.98c1.634-.14 2.055-.047 3.082.7l4.249 2.986c.7.513.934.653.934 1.213v16.378c0 1.026-.373 1.634-1.68 1.726l-15.458.934c-.98.047-1.448-.093-1.962-.747l-3.129-4.06c-.56-.747-.793-1.306-.793-1.96V2.667c0-.839.374-1.54 1.447-1.632z"/></svg>`;
const SVG_FOLDER = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>`;

export const MCP_PRESETS: readonly McpPreset[] = [
	// GitHub — verified 2026-07-05.
	// Docs: https://github.com/github/github-mcp-server (remote server section):
	// endpoint https://api.githubcopilot.com/mcp/ ; PAT supported via
	// `"Authorization": "Bearer ${input:github_mcp_pat}"` (OAuth optional, not
	// required). Live probe: no auth → 401; PAT-shaped bearer → 401
	// "AuthenticateToken authentication failed" (Bearer PATs are parsed).
	{
		id: "github",
		label: "GitHub",
		tagline: "Issues, pull requests, code search.",
		lede: "Chat can search code, read issues, and open pull requests in your repos.",
		kind: "hosted",
		url: "https://api.githubcopilot.com/mcp/",
		secretLabel: "Personal access token",
		helpUrl: "https://github.com/settings/personal-access-tokens",
		brandSvg: SVG_GITHUB,
	},
	// Linear — verified 2026-07-05.
	// Docs: https://linear.app/docs/mcp — endpoint https://mcp.linear.app/mcp ;
	// "The MCP server now supports passing OAuth token and API keys directly in
	// the `Authorization: Bearer <yourtoken>` header." Key created under
	// Settings → Account → Security & Access. Live probe: bogus bearer → 401.
	{
		id: "linear",
		label: "Linear",
		tagline: "Issues, projects, cycles.",
		lede: "Chat can look up and update issues, projects, and cycles.",
		kind: "hosted",
		url: "https://mcp.linear.app/mcp",
		secretLabel: "API key",
		helpUrl: "https://linear.app/settings/account/security",
		brandSvg: SVG_LINEAR,
	},
	// Stripe — verified 2026-07-05.
	// Docs: https://docs.stripe.com/mcp — endpoint https://mcp.stripe.com ; "If
	// your MCP client doesn't support OAuth, you can pass in a restricted API
	// key in the Authorisation header as a Bearer token." Live probe: bogus
	// bearer → 401.
	{
		id: "stripe",
		label: "Stripe",
		tagline: "Customers, payments, invoices.",
		lede: "Chat can look up customers, payments, and invoices.",
		kind: "hosted",
		url: "https://mcp.stripe.com",
		secretLabel: "Restricted API key",
		helpUrl: "https://dashboard.stripe.com/apikeys",
		brandSvg: SVG_STRIPE,
	},
	// Notion — verified 2026-07-05. Hosted mcp.notion.com is OAuth-only, so v1
	// ships Notion's official LOCAL server instead (static integration token).
	// Docs: https://github.com/makenotion/notion-mcp-server — `npx -y
	// @notionhq/notion-mcp-server` with env `NOTION_TOKEN` (ntn_… token from
	// https://www.notion.so/profile/integrations). npm: @notionhq/notion-mcp-server
	// v2.4.1, active.
	{
		id: "notion",
		label: "Notion",
		tagline: "Pages and databases.",
		lede: "Chat can read and edit your pages and databases.",
		kind: "local",
		command: "npx",
		args: ["-y", "@notionhq/notion-mcp-server"],
		secretLabel: "Integration token",
		secretEnvName: "NOTION_TOKEN",
		helpUrl: "https://www.notion.so/profile/integrations",
		brandSvg: SVG_NOTION,
	},
	// Filesystem — verified 2026-07-05. npm: @modelcontextprotocol/server-filesystem
	// v2026.7.4 (published 2026-07-04, not deprecated). Docs:
	// https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem —
	// allowed directories are positional args; the server expands a leading `~`
	// itself (expandHome), so the literal default works even though Mnema spawns
	// the command without a shell. No secret.
	{
		id: "filesystem",
		label: "Filesystem",
		tagline: "Read and write files in folders you choose.",
		lede: "Chat can read and write files in the folders you pick.",
		kind: "local",
		command: "npx",
		args: ["-y", "@modelcontextprotocol/server-filesystem", "~/Documents"],
		brandSvg: SVG_FOLDER,
	},
];

/**
 * Display label for a new connector from `preset`: the preset label, suffixed
 * "(2)", "(3)", … when that label is already taken (duplicates are allowed —
 * the id slugger independently suffixes the id).
 */
export function presetDisplayLabel(
	presetLabel: string,
	existingLabels: readonly string[],
): string {
	const taken = new Set(existingLabels.map((l) => l.trim()));
	if (!taken.has(presetLabel)) return presetLabel;
	let suffix = 2;
	while (taken.has(`${presetLabel} (${suffix})`)) suffix += 1;
	return `${presetLabel} (${suffix})`;
}

/**
 * Pre-fill the EXISTING McpServerConfig draft shape from a preset. The id
 * comes from `newMcpServerId` (stable slug — keys the keychain secret and the
 * `mcp__<id>__` tool prefix; never re-slugged). `enabled: true` because the
 * add flow immediately verifies the connection (plan: "add the draft
 * (enabled, autosave syncs it)"); callers flip it off for stdio-while-Node-
 * missing (slice 3).
 */
export function presetToDraft(
	preset: McpPreset,
	existing: readonly Pick<McpServerConfig, "id" | "label">[],
): McpServerConfig {
	return {
		id: newMcpServerId(
			preset.label,
			existing.map((s) => s.id),
		),
		label: presetDisplayLabel(
			preset.label,
			existing.map((s) => s.label),
		),
		enabled: true,
		transport: preset.kind === "hosted" ? "http" : "stdio",
		command: preset.kind === "local" ? (preset.command ?? null) : null,
		args: preset.kind === "local" ? [...(preset.args ?? [])] : [],
		env: [],
		url: preset.kind === "hosted" ? (preset.url ?? null) : null,
		secretEnvName: preset.kind === "local" ? (preset.secretEnvName ?? null) : null,
		enabledTools: null,
	};
}
