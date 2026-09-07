import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const rustPath = `${root}/crates/protocol/src/lib.rs`;
const outputPath = `${root}/crates/protocol/typescript.ts`;

function snakeCase(value) {
  return value.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
}

function splitTopLevel(value) {
  const parts = [];
  let start = 0;
  let depth = 0;
  let quoted = false;
  for (let index = 0; index < value.length; index += 1) {
    const character = value[index];
    if (character === '"' && value[index - 1] !== "\\") quoted = !quoted;
    if (quoted) continue;
    if ("<{[(".includes(character)) depth += 1;
    if (">}])".includes(character)) depth -= 1;
    if (character === "," && depth === 0) {
      parts.push(value.slice(start, index));
      start = index + 1;
    }
  }
  parts.push(value.slice(start));
  return parts.map((part) => part.trim()).filter(Boolean);
}

function matchingBrace(source, open) {
  let depth = 0;
  for (let index = open; index < source.length; index += 1) {
    if (source[index] === "{") depth += 1;
    if (source[index] === "}") depth -= 1;
    if (depth === 0) return index;
  }
  throw new Error("unclosed Rust declaration");
}

function attributes(value) {
  return [...value.matchAll(/#\[([^\]]*)\]/g)].map((match) => match[1]);
}

function renamed(attrs, fallback, renameAll) {
  const serde = attrs.find((attribute) => attribute.startsWith("serde")) ?? "";
  const explicit = serde.match(/rename\s*=\s*"([^"]+)"/);
  if (explicit) return explicit[1];
  return renameAll === "snake_case" ? snakeCase(fallback) : fallback;
}

function rustType(type) {
  const compact = type.replace(/\s+/g, "");
  const generic = (name) =>
    compact.startsWith(`${name}<`) && compact.endsWith(">")
      ? compact.slice(name.length + 1, -1)
      : null;
  const option = generic("Option");
  if (option !== null) return `${rustType(option)} | null`;
  const patch = generic("Patch");
  if (patch !== null) return `${rustType(patch)} | null`;
  const vector = generic("Vec");
  if (vector !== null) {
    const item = rustType(vector);
    return item.includes(" | ") ? `(${item})[]` : `${item}[]`;
  }
  if (["i64", "u64", "usize", "f64"].includes(compact)) return "number";
  if (compact === "String" || compact === "&str") return "string";
  if (compact === "bool") return "boolean";
  if (compact === "serde_json::Value") return "unknown";
  return compact;
}

function parseFields(body, owner) {
  return splitTopLevel(body).map((part) => {
    const attrs = attributes(part);
    const field = part.replace(/(?:#\[[^\]]*\]\s*)+/g, "").trim();
    const match = field.match(/^(?:pub\s+)?([A-Za-z0-9_]+)\s*:\s*([\s\S]+)$/);
    if (!match) throw new Error(`unsupported field in ${owner}: ${field}`);
    const [, name, type] = match;
    const serde = attrs.join(" ");
    const optional =
      serde.includes("skip_serializing_if") ||
      ((owner.endsWith("Request") || owner.endsWith("Query")) &&
        (type.replace(/\s/g, "").startsWith("Option<") || serde.includes("default")));
    return { name, type: rustType(type), optional, flatten: serde.includes("flatten") };
  });
}

function renderFields(fields, indent = "  ") {
  return fields
    .map(({ name, type, optional }) => `${indent}${name}${optional ? "?" : ""}: ${type};`)
    .join("\n");
}

function renderStruct(name, generic, body) {
  if (name === "Patch") return "";
  const fields = parseFields(body, name);
  const flattened = fields.filter((field) => field.flatten);
  const ordinary = fields.filter((field) => !field.flatten);
  if (flattened.length) {
    const left = flattened.map((field) => field.type).join(" & ");
    return `export type ${name}${generic} = ${left} & {\n${renderFields(ordinary)}\n};`;
  }
  return `export interface ${name}${generic} {\n${renderFields(ordinary)}\n}`;
}

function renderEnum(name, declarationAttrs, body) {
  if (name === "Patch") return "";
  const serde = declarationAttrs.join(" ");
  const renameAll = serde.match(/rename_all\s*=\s*"([^"]+)"/)?.[1];
  const tag = serde.match(/tag\s*=\s*"([^"]+)"/)?.[1];
  const content = serde.match(/content\s*=\s*"([^"]+)"/)?.[1];
  const variants = splitTopLevel(body).map((part) => {
    const attrs = attributes(part);
    const clean = part.replace(/(?:#\[[^\]]*\]\s*)+/g, "").trim();
    const nameMatch = clean.match(/^([A-Za-z0-9_]+)/);
    if (!nameMatch) throw new Error(`unsupported variant in ${name}: ${clean}`);
    const variant = nameMatch[1];
    const wireName = renamed(attrs, variant, renameAll);
    const tuple = clean.match(/^[A-Za-z0-9_]+\s*\(([\s\S]*)\)$/);
    const record = clean.match(/^[A-Za-z0-9_]+\s*\{([\s\S]*)\}$/);
    return { wireName, tuple: tuple?.[1], fields: record ? parseFields(record[1], name) : null };
  });
  if (!tag) {
    return `export type ${name} = ${variants.map(({ wireName }) => `"${wireName}"`).join(" | ")};`;
  }
  const members = variants.map(({ wireName, tuple, fields }) => {
    const values = [`${tag}: "${wireName}"`];
    if (tuple) values.push(`${content}: ${rustType(tuple)}`);
    if (fields) values.push(...fields.map(({ name: field, type }) => `${field}: ${type}`));
    return `  | { ${values.join("; ")} }`;
  });
  return `export type ${name} =\n${members.join("\n")};`;
}

export function generate(source) {
  source = source.split("#[cfg(test)]")[0];
  const declarations = [];
  const seen = [];
  const pattern = /((?:#\[[^\]]*\]\s*)*)pub\s+(struct|enum)\s+([A-Za-z0-9_]+)(\s*<[^>{}]+>)?\s*\{/g;
  for (let match = pattern.exec(source); match; match = pattern.exec(source)) {
    const open = source.indexOf("{", match.index + match[0].length - 1);
    const close = matchingBrace(source, open);
    const [, rawAttrs, kind, name, rawGeneric = ""] = match;
    seen.push(`${kind}:${name}`);
    const attrs = attributes(rawAttrs);
    const generic = rawGeneric.replace(/\s/g, "");
    const body = source.slice(open + 1, close);
    declarations.push(
      kind === "struct"
        ? renderStruct(name, generic, body)
        : renderEnum(name, attrs, body),
    );
    if (name === "Message") declarations.push("export type BoardMessage = Message;");
    pattern.lastIndex = close + 1;
  }
  const publicDeclarations = [...source.matchAll(/pub\s+(struct|enum)\s+([A-Za-z0-9_]+)/g)]
    .map((match) => `${match[1]}:${match[2]}`);
  if (seen.join("\n") !== publicDeclarations.join("\n")) {
    throw new Error("unsupported public Rust wire declaration");
  }
  if (/pub\s+type\s+/.test(source)) {
    throw new Error("public Rust type aliases need generator support");
  }
  return `// Generated from crates/protocol/src/lib.rs. Do not edit.\n${declarations.filter(Boolean).join("\n\n")}\n`;
}

export async function check(output = outputPath) {
  const expected = generate(await readFile(rustPath, "utf8"));
  return expected === await readFile(output, "utf8").catch(() => "");
}

async function main() {
  const generated = generate(await readFile(rustPath, "utf8"));
  if (process.argv.includes("--check")) {
    if (generated !== await readFile(outputPath, "utf8").catch(() => "")) {
      console.error("TypeScript protocol contract is stale; run npm run generate:protocol.");
      process.exitCode = 1;
    }
    return;
  }
  await writeFile(outputPath, generated);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
