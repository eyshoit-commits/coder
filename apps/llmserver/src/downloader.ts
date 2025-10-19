import fs from "node:fs";
import path from "node:path";
import { downloadFile } from "@huggingface/hub";
import { MODEL_CATALOG, getModelMetadata } from "./catalog";
import { ServerConfig } from "./config";

export interface DownloadProgress {
  model: string;
  file: string;
  size: number;
}

export async function downloadModel(config: ServerConfig, modelName: string): Promise<DownloadProgress> {
  const metadata = getModelMetadata(modelName);
  if (!metadata) {
    throw new Error(`Model '${modelName}' is not supported`);
  }
  const targetDir = path.join(config.modelsDir, metadata.name);
  const targetPath = path.join(targetDir, metadata.file);
  if (!fs.existsSync(targetDir)) {
    fs.mkdirSync(targetDir, { recursive: true });
  }
  if (fs.existsSync(targetPath)) {
    const stats = fs.statSync(targetPath);
    return { model: metadata.name, file: targetPath, size: stats.size };
  }

  const tempPath = `${targetPath}.download`;
  const download = await downloadFile({
    repo: metadata.huggingFaceRepo,
    file: metadata.file,
    localFile: tempPath
  });
  if (!download) {
    throw new Error(`Failed to download ${metadata.file} from HuggingFace repo ${metadata.huggingFaceRepo}`);
  }
  fs.renameSync(tempPath, targetPath);
  const stats = fs.statSync(targetPath);
  return { model: metadata.name, file: targetPath, size: stats.size };
}

export function listAvailableDownloads(): DownloadProgress[] {
  return MODEL_CATALOG.map((metadata) => {
    const file = `${metadata.huggingFaceRepo}/${metadata.file}`;
    return { model: metadata.name, file, size: 0 };
  });
}
