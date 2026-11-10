// 读写 PE 文件里的 VS_VERSIONINFO（RT_VERSION）资源。
// 用途：任务管理器/资源管理器的"名称"列取的是 FileDescription，而不是文件名；
// 打包时把服务组件的描述改成对外的产品名，文件名保持稳定（服务注册路径、校验都依赖它）。
import { readFileSync, writeFileSync } from "node:fs";

const RT_VERSION = 16;
const align4 = (n) => (n + 3) & ~3;

function fail(message) {
  throw new Error(message);
}

function parsePe(buf) {
  if (buf.length < 0x40 || buf.readUInt16LE(0) !== 0x5a4d)
    fail("不是有效的 PE 文件");
  const lfanew = buf.readUInt32LE(0x3c);
  if (buf.readUInt32LE(lfanew) !== 0x00004550) fail("不是有效的 PE 文件");
  const coff = lfanew + 4;
  const sectionsCount = buf.readUInt16LE(coff + 2);
  const optionalSize = buf.readUInt16LE(coff + 16);
  const optional = coff + 20;
  const magic = buf.readUInt16LE(optional);
  if (magic !== 0x10b && magic !== 0x20b) fail("未知的 PE 可选头");
  const directories = optional + (magic === 0x20b ? 0x70 : 0x60);
  const sectionTable = optional + optionalSize;
  const sections = [];
  for (let index = 0; index < sectionsCount; index += 1) {
    const offset = sectionTable + index * 40;
    sections.push({
      name: buf.toString("latin1", offset, offset + 8).replace(/\0.*$/, ""),
      virtualSize: buf.readUInt32LE(offset + 8),
      virtualAddress: buf.readUInt32LE(offset + 12),
      rawSize: buf.readUInt32LE(offset + 16),
      rawPointer: buf.readUInt32LE(offset + 20),
    });
  }
  return { buf, directories, sections };
}

function rvaToOffset(pe, rva) {
  for (const section of pe.sections) {
    const span = Math.max(section.virtualSize, section.rawSize);
    if (rva >= section.virtualAddress && rva < section.virtualAddress + span) {
      return section.rawPointer + (rva - section.virtualAddress);
    }
  }
  fail(`RVA 0x${rva.toString(16)} 不在任何节内`);
}

function readResourceDirectory(buf, offset) {
  const named = buf.readUInt16LE(offset + 12);
  const ids = buf.readUInt16LE(offset + 14);
  const entries = [];
  for (let index = 0; index < named + ids; index += 1) {
    const entry = offset + 16 + index * 8;
    entries.push({
      id: buf.readUInt32LE(entry),
      target: buf.readUInt32LE(entry + 4),
    });
  }
  return entries;
}

// 返回 [{ type, id, lang, dataEntryOffset, dataRva, size }]
function readVersionEntries(pe) {
  const { buf, directories } = pe;
  const resourceRva = buf.readUInt32LE(directories + 2 * 8);
  if (!resourceRva) fail("文件没有资源节");
  const resourceBase = rvaToOffset(pe, resourceRva);
  const result = [];
  for (const typeEntry of readResourceDirectory(buf, resourceBase)) {
    if ((typeEntry.target & 0x80000000) === 0) continue;
    if (typeEntry.id !== RT_VERSION) continue;
    const typeDir = resourceBase + (typeEntry.target & 0x7fffffff);
    for (const nameEntry of readResourceDirectory(buf, typeDir)) {
      if ((nameEntry.target & 0x80000000) === 0) continue;
      const nameDir = resourceBase + (nameEntry.target & 0x7fffffff);
      for (const langEntry of readResourceDirectory(buf, nameDir)) {
        if ((langEntry.target & 0x80000000) !== 0) continue;
        const dataEntryOffset = resourceBase + langEntry.target;
        const dataRva = buf.readUInt32LE(dataEntryOffset);
        const size = buf.readUInt32LE(dataEntryOffset + 4);
        result.push({
          type: typeEntry.id,
          id: nameEntry.id,
          lang: langEntry.id,
          dataEntryOffset,
          dataRva,
          size,
        });
      }
    }
  }
  return result;
}

function readVersionString(blob, key) {
  const needle = Buffer.from(`${key}\0`, "utf16le");
  const index = blob.indexOf(needle);
  if (index < 0) return null;
  // 字符串值紧跟在键之后、按 4 字节对齐的位置，以 NUL 结尾。
  let cursor = index + needle.length;
  cursor = align4(cursor);
  const values = [];
  for (let offset = cursor; offset + 1 < blob.length; offset += 2) {
    const unit = blob.readUInt16LE(offset);
    if (unit === 0) break;
    values.push(unit);
  }
  return Buffer.from(new Uint16Array(values).buffer).toString("utf16le") || "";
}

function block({ type = 1, valueLength, key, value, children = [] }) {
  const keyBuffer = Buffer.from(`${key}\0`, "utf16le");
  const keyLength = align4(6 + keyBuffer.length);
  const valueBuffer = value ? Buffer.from(value) : Buffer.alloc(0);
  const body = Buffer.concat([
    Buffer.alloc(6),
    keyBuffer,
    Buffer.alloc(keyLength - 6 - keyBuffer.length),
    valueBuffer,
    ...children,
  ]);
  body.writeUInt16LE(body.length, 0);
  body.writeUInt16LE(valueLength, 2);
  body.writeUInt16LE(type, 4);
  return body;
}

function stringEntry(key, value) {
  const keyBuffer = Buffer.from(`${key}\0`, "utf16le");
  const valueBuffer = Buffer.from(`${value}\0`, "utf16le");
  const keyLength = align4(6 + keyBuffer.length);
  const body = Buffer.concat([
    Buffer.alloc(6),
    keyBuffer,
    Buffer.alloc(keyLength - 6 - keyBuffer.length),
    valueBuffer,
    Buffer.alloc(align4(valueBuffer.length) - valueBuffer.length),
  ]);
  body.writeUInt16LE(body.length, 0);
  body.writeUInt16LE(valueBuffer.length / 2, 2);
  body.writeUInt16LE(1, 4);
  return body;
}

function fixedFileInfo(version) {
  const [major, minor, patch] = String(version).split(".").map(Number);
  const buffer = Buffer.alloc(52);
  buffer.writeUInt32LE(0xfeef04bd, 0);
  buffer.writeUInt32LE(0x00010000, 4);
  buffer.writeUInt32LE(((major || 0) << 16) | (minor || 0), 8);
  buffer.writeUInt32LE(((patch || 0) << 16) | 0, 12);
  buffer.writeUInt32LE(((major || 0) << 16) | (minor || 0), 16);
  buffer.writeUInt32LE(((patch || 0) << 16) | 0, 20);
  buffer.writeUInt32LE(0x3f, 24);
  buffer.writeUInt32LE(0, 28);
  buffer.writeUInt32LE(0x40004, 32);
  buffer.writeUInt32LE(1, 36);
  buffer.writeUInt32LE(0, 40);
  buffer.writeUInt32LE(0, 44);
  buffer.writeUInt32LE(0, 48);
  return buffer;
}

export function buildVersionInfo({
  language = 0x0409,
  codePage = 0x04b0,
  version = "0.0.0",
  fileDescription,
  productName,
  companyName,
  legalCopyright,
  originalFilename,
  internalName,
}) {
  const table = [
    ["CompanyName", companyName],
    ["FileDescription", fileDescription],
    ["FileVersion", version],
    ["InternalName", internalName],
    ["LegalCopyright", legalCopyright],
    ["OriginalFilename", originalFilename],
    ["ProductName", productName],
    ["ProductVersion", version],
  ]
    .filter(([, value]) => typeof value === "string" && value.length > 0)
    .map(([key, value]) => stringEntry(key, value));
  const languageKey = `${language.toString(16).padStart(4, "0")}${codePage
    .toString(16)
    .padStart(4, "0")}`.toUpperCase();
  const stringTable = block({
    valueLength: 0,
    key: languageKey,
    children: table,
  });
  const stringFileInfo = block({
    valueLength: 0,
    key: "StringFileInfo",
    children: [stringTable],
  });
  return block({
    type: 0,
    valueLength: 52,
    key: "VS_VERSION_INFO",
    value: fixedFileInfo(version),
    children: [stringFileInfo],
  });
}

export function inspect(file) {
  const pe = parsePe(readFileSync(file));
  return readVersionEntries(pe).map((entry) => {
    const offset = rvaToOffset(pe, entry.dataRva);
    const blob = pe.buf.subarray(offset, offset + entry.size);
    return {
      language: entry.lang,
      size: entry.size,
      fileDescription: readVersionString(blob, "FileDescription"),
      productName: readVersionString(blob, "ProductName"),
      fileVersion: readVersionString(blob, "FileVersion"),
      companyName: readVersionString(blob, "CompanyName"),
    };
  });
}

export function setFileDescription(file, description, { version } = {}) {
  const pe = parsePe(readFileSync(file));
  const entries = readVersionEntries(pe);
  if (entries.length === 0) fail("文件没有版本资源，无法改写");
  const patched = entries.map((entry) => {
    const offset = rvaToOffset(pe, entry.dataRva);
    const current = pe.buf.subarray(offset, offset + entry.size);
    const existingVersion =
      version || readVersionString(current, "FileVersion") || "0.0.0";
    const blob = buildVersionInfo({
      language: entry.lang || 0x0409,
      version: existingVersion,
      fileDescription: description,
      productName: readVersionString(current, "ProductName") || undefined,
      companyName: readVersionString(current, "CompanyName") || undefined,
      legalCopyright: readVersionString(current, "LegalCopyright") || undefined,
      internalName: readVersionString(current, "InternalName") || undefined,
    });
    if (blob.length > entry.size) {
      fail(
        `新版本资源 ${blob.length} 字节超过原有 ${entry.size} 字节，无法原地写入`,
      );
    }
    return { entry, offset, blob };
  });
  for (const { entry, offset, blob } of patched) {
    pe.buf.fill(0, offset, offset + entry.size);
    blob.copy(pe.buf, offset);
    pe.buf.writeUInt32LE(blob.length, entry.dataEntryOffset + 4);
  }
  writeFileSync(file, pe.buf);
  return patched.map(({ blob }) => blob.length);
}

if (process.argv[1] && process.argv[1].endsWith("pe-version.mjs")) {
  const [, , action, file, ...rest] = process.argv;
  if (action === "inspect") {
    console.log(JSON.stringify(inspect(file), null, 2));
  } else if (action === "set") {
    const description = rest[0];
    console.log(`写入 ${file}：FileDescription = ${description}`);
    console.log(
      `新资源长度：${setFileDescription(file, description).join(", ")}`,
    );
  } else {
    console.error("用法：node scripts/pe-version.mjs inspect|set <exe> [描述]");
    process.exitCode = 1;
  }
}
