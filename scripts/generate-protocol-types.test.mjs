import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { check, generate } from "./generate-protocol-types.mjs";

test("the drift check rejects a stale checked-in contract", async () => {
  const directory = await mkdtemp(join(tmpdir(), "board-protocol-"));
  const output = join(directory, "typescript.ts");
  try {
    await writeFile(output, "// stale\n");
    assert.equal(await check(output), false);

    const source = await readFile(
      new URL("../crates/protocol/src/lib.rs", import.meta.url),
      "utf8",
    );
    await writeFile(output, generate(source));
    assert.equal(await check(output), true);
    const changed = source.replace(
      "pub struct ErrorResponse {",
      "pub struct ErrorResponse {\n    pub code: i64,",
    );
    assert.match(generate(changed), /code: number;/);
    assert.notEqual(generate(changed), generate(source));
  } finally {
    await rm(directory, { recursive: true });
  }
});
