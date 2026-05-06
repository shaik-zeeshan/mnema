import { convertFileSrc } from "@tauri-apps/api/core";

type FramePreviewFetchDependencies = {
  convertFileSrcImpl?: (filePath: string) => string;
  fetchImpl?: typeof fetch;
};

export function framePreviewAssetUrl(
  filePath: string,
  deps: Pick<FramePreviewFetchDependencies, "convertFileSrcImpl"> = {},
): string {
  const convert = deps.convertFileSrcImpl ?? convertFileSrc;
  return convert(filePath);
}

export async function readFramePreviewBytes(
  filePath: string,
  deps: FramePreviewFetchDependencies = {},
): Promise<Uint8Array> {
  const fetchImpl = deps.fetchImpl ?? fetch;
  const assetUrl = framePreviewAssetUrl(filePath, deps);
  const response = await fetchImpl(assetUrl);
  if (!response.ok) {
    throw new Error(`frame preview fetch failed: ${response.status} ${response.statusText}`.trim());
  }
  return new Uint8Array(await response.arrayBuffer());
}
