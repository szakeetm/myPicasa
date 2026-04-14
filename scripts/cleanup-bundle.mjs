import fs from "node:fs/promises";
import os from "node:os";

if (os.platform() !== "darwin") {
  process.exit(0);
}

const targets = [
  "src-tauri/target/release/bundle/dmg",
  "src-tauri/target/release/bundle/macos",
];

for (const dir of targets) {
  let entries = [];
  try {
    entries = await fs.readdir(dir);
  } catch {
    continue;
  }

  for (const entry of entries) {
    const isDmg = entry.endsWith(".dmg");
    const isRwDmg = entry.startsWith("rw.") && entry.endsWith(".dmg");
    if (!isDmg && !isRwDmg) {
      continue;
    }

    await fs.rm(`${dir}/${entry}`, { force: true });
  }
}