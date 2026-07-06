// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { beforeEach, describe, expect, mock, test } from "bun:test";

// Mock the Tauri dialog BEFORE importing the SUT so `removeMcpServer` binds the
// mock at import time (openCapturedUrl.test.ts pattern). bun module mocks fix
// the export-name set process-wide, so export the sibling dialog names other
// suites/modules import (`message`, `ask`) alongside the `confirm` under test.
const confirm = mock(async (_msg: string, _opts?: unknown): Promise<boolean> => true);
const message = mock(async (_msg: string, _opts?: unknown): Promise<void> => {});
const ask = mock(async (_msg: string, _opts?: unknown): Promise<boolean> => false);
mock.module("@tauri-apps/plugin-dialog", () => ({ confirm, message, ask }));

const { createMcpConnectorActions } = await import("./controller-mcp");
const { MCP_PRESETS } = await import("./mcp-presets");

const githubPreset = MCP_PRESETS.find((p) => p.id === "github");
// A local (stdio) preset for the stdio-while-Node-missing override case. Notion
// was a local npx server pre-ADR-0051; it is now hosted OAuth, so the stdio path
// is exercised via filesystem.
const filesystemPreset = MCP_PRESETS.find((p) => p.id === "filesystem");

// Minimal fakes for the injected stores — only the slice controller-mcp reads.
// The snapshot is derived from the draft list so add/remove makes the domain
// dirty and a (fake) successful save makes it clean again, matching how the
// real recording store's ai_runtime snapshot behaves.
function makeDeps() {
  const rec = {
    draftMcpServers: [],
    savingRecDomains: { ai_runtime: false },
    lastSavedRecSnapshots: { ai_runtime: null },
    recError: null,
    buildRecDomainSnapshot: () => JSON.stringify(rec.draftMcpServers.map((s) => s.id)),
  };
  const aiRuntime = {
    refreshMcpServerSecretPresence: mock(async () => {}),
    clearSecretForRemovedMcpServer: mock(async (_id: string) => {}),
  };
  // Default save behavior: the direct save lands, so the baseline snapshot
  // catches up to the live draft state (a clean flush).
  const saveAiRuntime = mock(async () => {
    rec.lastSavedRecSnapshots.ai_runtime = rec.buildRecDomainSnapshot("ai_runtime");
  });
  const actions = createMcpConnectorActions({ rec, aiRuntime, saveAiRuntime });
  return { rec, aiRuntime, saveAiRuntime, actions };
}

beforeEach(() => {
  confirm.mockReset();
  confirm.mockImplementation(async () => true);
});

describe("addMcpServerFromPreset", () => {
  test("prepends the preset draft and refreshes secret presence", () => {
    const { rec, aiRuntime, actions } = makeDeps();

    const id = actions.addMcpServerFromPreset(githubPreset);

    expect(id).toBe("github");
    expect(rec.draftMcpServers).toHaveLength(1);
    expect(rec.draftMcpServers[0]).toMatchObject({
      id: "github",
      label: "GitHub",
      enabled: true,
      transport: "http",
      url: githubPreset.url,
    });
    expect(aiRuntime.refreshMcpServerSecretPresence).toHaveBeenCalledTimes(1);
  });

  test("second add of the same preset lands at the TOP with a collision-free id", () => {
    const { rec, actions } = makeDeps();

    actions.addMcpServerFromPreset(githubPreset);
    const secondId = actions.addMcpServerFromPreset(githubPreset);

    expect(secondId).toBe("github-2");
    expect(rec.draftMcpServers.map((s) => s.id)).toEqual(["github-2", "github"]);
  });

  test("overrides win over the preset draft (stdio-while-Node-missing starts disabled)", () => {
    const { rec, actions } = makeDeps();

    actions.addMcpServerFromPreset(filesystemPreset, { enabled: false });

    expect(rec.draftMcpServers[0]).toMatchObject({
      enabled: false,
      transport: "stdio",
      command: filesystemPreset.command,
    });
  });
});

describe("removeMcpServer", () => {
  test("confirmed remove deletes the draft and clears its keychain secret", async () => {
    const { rec, aiRuntime, actions } = makeDeps();
    actions.addMcpServerFromPreset(githubPreset);

    await actions.removeMcpServer("github");

    expect(confirm).toHaveBeenCalledTimes(1);
    expect(rec.draftMcpServers).toHaveLength(0);
    expect(aiRuntime.clearSecretForRemovedMcpServer).toHaveBeenCalledWith("github");
  });

  test("user declines the dialog -> connector kept, secret untouched", async () => {
    const { rec, aiRuntime, actions } = makeDeps();
    actions.addMcpServerFromPreset(githubPreset);
    confirm.mockImplementation(async () => false);

    await actions.removeMcpServer("github");

    expect(rec.draftMcpServers.map((s) => s.id)).toEqual(["github"]);
    expect(aiRuntime.clearSecretForRemovedMcpServer).not.toHaveBeenCalled();
  });

  test("confirm:false (verify-on-add rollback) removes silently — no dialog", async () => {
    const { rec, aiRuntime, actions } = makeDeps();
    actions.addMcpServerFromPreset(githubPreset);

    await actions.removeMcpServer("github", { confirm: false });

    expect(confirm).not.toHaveBeenCalled();
    expect(rec.draftMcpServers).toHaveLength(0);
    expect(aiRuntime.clearSecretForRemovedMcpServer).toHaveBeenCalledWith("github");
  });

  test("unknown id is a no-op (no dialog, no secret clear)", async () => {
    const { rec, aiRuntime, actions } = makeDeps();
    actions.addMcpServerFromPreset(githubPreset);

    await actions.removeMcpServer("nope");

    expect(confirm).not.toHaveBeenCalled();
    expect(rec.draftMcpServers).toHaveLength(1);
    expect(aiRuntime.clearSecretForRemovedMcpServer).not.toHaveBeenCalled();
  });
});

describe("flushAiRuntimeSave", () => {
  test("resolves when the direct save lands the current draft state", async () => {
    const { saveAiRuntime, actions } = makeDeps();
    actions.addMcpServerFromPreset(githubPreset);

    await actions.flushAiRuntimeSave();

    expect(saveAiRuntime).toHaveBeenCalledTimes(1);
  });

  test("throws rec.recError when the save leaves the domain dirty (null baseline)", async () => {
    const { rec, saveAiRuntime, actions } = makeDeps();
    actions.addMcpServerFromPreset(githubPreset);
    // Failed save: the baseline never catches up and the store reports why.
    saveAiRuntime.mockImplementation(async () => {
      rec.recError = "backend exploded";
    });

    await expect(actions.flushAiRuntimeSave()).rejects.toThrow("backend exploded");
  });

  test("throws the generic copy when dirty with no recError (stale baseline)", async () => {
    const { rec, saveAiRuntime, actions } = makeDeps();
    actions.addMcpServerFromPreset(githubPreset);
    // The save persisted a PRE-add snapshot (armed backoff / raced autosave):
    // baseline is non-null but stale against the live draft.
    saveAiRuntime.mockImplementation(async () => {
      rec.lastSavedRecSnapshots.ai_runtime = JSON.stringify([]);
    });

    await expect(actions.flushAiRuntimeSave()).rejects.toThrow(
      "Could not save the connector settings — try again.",
    );
  });

  test("waits out an in-flight ai_runtime save before saving directly", async () => {
    const { rec, saveAiRuntime, actions } = makeDeps();
    actions.addMcpServerFromPreset(githubPreset);
    rec.savingRecDomains.ai_runtime = true;
    let savingWhenSaveRan: boolean | null = null;
    saveAiRuntime.mockImplementation(async () => {
      savingWhenSaveRan = rec.savingRecDomains.ai_runtime;
      rec.lastSavedRecSnapshots.ai_runtime = rec.buildRecDomainSnapshot("ai_runtime");
    });
    setTimeout(() => {
      rec.savingRecDomains.ai_runtime = false;
    }, 120);

    await actions.flushAiRuntimeSave();

    expect(savingWhenSaveRan).toBe(false);
  });
});
